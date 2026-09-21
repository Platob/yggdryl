//! `rust/src/iceberg/scan.rs`: the planning a scan does before it reads.
//!
//! A plan is what a caller receives, but every refusal and every pruning
//! decision inside it is a rule of its own - a delete manifest that must fail
//! before a file is opened, a partition tuple that must match its spec, a row
//! id that must inherit contiguously, a bound that must decode under the
//! schema it evolved to. Those steps are forwarded through
//! `yggdryl::internals`; what a caller observes of a planned scan is pinned in
//! `rust/tests/iceberg/mod_.rs`.

mod delete_tests {
    use smol_str::{SmolStr, format_smolstr};

    use yggdryl::iceberg::{
        DataFile, ManifestContent, ManifestEntry, ManifestFile, PartitionField, PartitionSpec,
        ScanPlan, Transform,
    };
    use yggdryl::internals::iceberg_scan::{
        identity_column, inherit_first_row_id, plan, validate_data_entry,
    };
    use yggdryl::{DataType, Error, Field, StructType};

    /// The malformed-document error the planner's own caller reports.
    fn invalid(reason: SmolStr) -> Error {
        Error::Codec {
            format: "iceberg",
            position: 0,
            reason,
        }
    }

    fn manifest(content: ManifestContent) -> ManifestFile {
        ManifestFile {
            manifest_path: "metadata/delete-manifest.avro".into(),
            manifest_length: 128,
            partition_spec_id: 0,
            content,
            sequence_number: 2,
            min_sequence_number: 1,
            added_snapshot_id: 7,
            added_files_count: Some(1),
            existing_files_count: Some(0),
            deleted_files_count: Some(0),
            added_rows_count: Some(1),
            existing_rows_count: Some(0),
            deleted_rows_count: Some(0),
            partitions: Vec::new(),
            key_metadata: None,
            first_row_id: None,
        }
    }

    fn schema() -> Field {
        StructType::from_fields([DataType::Int64.required_field("id")])
            .map(DataType::from)
            .unwrap()
            .required_field("row")
    }

    fn entry(content: i32) -> ManifestEntry {
        ManifestEntry::added(
            7,
            DataFile {
                content,
                file_path: "data/part.parquet".into(),
                mime_type: yggdryl::MimeType::PARQUET,
                record_count: 1,
                file_size_in_bytes: 128,
                ..DataFile::default()
            },
        )
    }

    fn assert_unsupported(error: Error, kind: &str) {
        let Error::Iceberg { reason, source } = error else {
            panic!("expected a typed Iceberg unsupported error, got {error}");
        };
        assert!(source.is_none());
        assert!(reason.contains(kind), "{reason}");
    }

    #[test]
    fn live_delete_manifests_fail_before_any_file_is_read() {
        let error = plan(
            &[manifest(ManifestContent::Deletes)],
            &|_| Ok(PartitionSpec::unpartitioned()),
            &|_| panic!("delete manifest must not be opened as row data"),
            &[],
            &schema(),
            true,
        )
        .unwrap_err();
        assert_unsupported(error, "delete manifest");
    }

    #[test]
    fn delete_manifests_with_unknown_counts_are_not_assumed_empty() {
        let mut unknown = manifest(ManifestContent::Deletes);
        unknown.added_files_count = None;
        unknown.existing_files_count = None;
        let error = plan(
            &[unknown],
            &|_| Ok(PartitionSpec::unpartitioned()),
            &|_| panic!("unknown delete counts must fail before the manifest is opened"),
            &[],
            &schema(),
            true,
        )
        .unwrap_err();
        assert_unsupported(error, "delete manifest");
    }

    #[test]
    fn delete_manifests_without_live_files_are_inert() {
        let mut deleted = manifest(ManifestContent::Deletes);
        deleted.added_files_count = Some(0);
        deleted.deleted_files_count = Some(1);
        deleted.added_rows_count = Some(0);
        deleted.deleted_rows_count = Some(1);
        let planned = plan(
            &[deleted],
            &|_| panic!("an inert delete manifest has no spec to resolve"),
            &|_| panic!("an inert delete manifest has no entries to read"),
            &[],
            &schema(),
            true,
        )
        .unwrap();
        assert_eq!(planned, ScanPlan::default());
    }

    #[test]
    fn missing_partition_specs_fail_before_pruning() {
        let error = plan(
            &[manifest(ManifestContent::Data)],
            &|spec_id| {
                Err(invalid(format_smolstr!(
                    "expected partition spec id {spec_id}, got none"
                )))
            },
            &|_| panic!("a manifest with an unresolved spec must not be opened"),
            &[],
            &schema(),
            true,
        )
        .unwrap_err()
        .to_string();
        assert!(
            error.contains("expected partition spec id 0, got none"),
            "{error}"
        );
    }

    #[test]
    fn delete_files_hidden_in_data_manifests_are_rejected() {
        let manifest = manifest(ManifestContent::Data);
        for (content, kind) in [(1, "position-delete"), (2, "equality-delete")] {
            let error =
                validate_data_entry(&entry(content), &manifest, &PartitionSpec::unpartitioned())
                    .unwrap_err();
            assert_unsupported(error, kind);
        }
    }

    #[test]
    fn partition_tuples_must_match_the_referenced_spec() {
        let spec = PartitionSpec {
            spec_id: 3,
            fields: vec![PartitionField::identity(1, 1_000, "id")],
        };
        let error = validate_data_entry(&entry(0), &manifest(ManifestContent::Data), &spec)
            .unwrap_err()
            .to_string();
        assert!(
            error.contains("expected 1 partition values for spec 3, got 0"),
            "{error}"
        );
    }

    #[test]
    fn unknown_transforms_never_supply_pruning_bounds() {
        let mut schema = schema();
        yggdryl::iceberg::assign_field_ids(&mut schema, 1).unwrap();
        let spec = PartitionSpec {
            spec_id: 3,
            fields: vec![PartitionField {
                source_id: 1,
                field_id: 1_000,
                name: "id_opaque".into(),
                transform: Transform::Unknown,
            }],
        };
        assert!(identity_column(&spec, 0, &schema).is_none());
    }

    #[test]
    fn data_files_inherit_contiguous_row_ids_without_moving_explicit_ids() {
        let mut cursor = Some(10);
        let mut first = entry(0);
        first.data_file.record_count = 3;
        inherit_first_row_id(&mut first, &mut cursor).unwrap();
        assert_eq!(first.data_file.first_row_id, Some(10));
        assert_eq!(cursor, Some(13));

        let mut explicit = entry(0);
        explicit.data_file.first_row_id = Some(100);
        explicit.data_file.record_count = 7;
        inherit_first_row_id(&mut explicit, &mut cursor).unwrap();
        assert_eq!(explicit.data_file.first_row_id, Some(100));
        assert_eq!(cursor, Some(13));

        let mut last = entry(0);
        last.data_file.record_count = 2;
        inherit_first_row_id(&mut last, &mut cursor).unwrap();
        assert_eq!(last.data_file.first_row_id, Some(13));
        assert_eq!(cursor, Some(15));
    }

    #[test]
    fn data_files_keep_null_row_ids_without_a_manifest_range() {
        let mut entry = entry(0);
        let mut cursor = None;
        inherit_first_row_id(&mut entry, &mut cursor).unwrap();
        assert_eq!(entry.data_file.first_row_id, None);
        assert_eq!(cursor, None);
    }

    #[test]
    fn row_id_overflow_fails_without_mutating_the_file_or_cursor() {
        let mut entry = entry(0);
        entry.data_file.record_count = 2;
        let mut cursor = Some(i64::MAX);
        let error = inherit_first_row_id(&mut entry, &mut cursor)
            .unwrap_err()
            .to_string();
        assert!(error.contains("row id overflow"), "{error}");
        assert_eq!(entry.data_file.first_row_id, None);
        assert_eq!(cursor, Some(i64::MAX));
    }
}

mod bound_tests {
    use yggdryl::iceberg::{DataFile, PartitionSpec};
    use yggdryl::internals::iceberg_scan::{conjuncts, file_bounds, file_residual};
    use yggdryl::{DataType, Field, Filter, Scalar, StructType, Term};

    fn schema(dtype: DataType) -> Field {
        let mut schema = StructType::from_fields([dtype.required_field("value")])
            .map(DataType::from)
            .unwrap()
            .required_field("row");
        yggdryl::iceberg::assign_field_ids(&mut schema, 1).unwrap();
        schema
    }

    fn residual(
        dtype: DataType,
        lower: Vec<u8>,
        upper: Vec<u8>,
        value: Scalar,
    ) -> Option<Vec<usize>> {
        let schema = schema(dtype);
        let file = DataFile {
            record_count: 1,
            lower_bounds: vec![(1, lower)],
            upper_bounds: vec![(1, upper)],
            ..DataFile::default()
        };
        let filter = Filter::new(Term::column("value").eq(Term::literal(value)));
        let conjuncts = conjuncts(&schema, &filter).unwrap();
        file_residual(
            &file_bounds(&file, &PartitionSpec::unpartitioned(), &schema),
            &conjuncts,
        )
    }

    #[test]
    fn promoted_bounds_prune_under_the_current_schema_type() {
        let int = 37_i32.to_le_bytes().to_vec();
        assert_eq!(
            residual(DataType::Int64, int.clone(), int, Scalar::from(37)),
            Some(Vec::new()),
            "an Int bound evolved to Long proves the matching value"
        );

        let float = 1.5_f32.to_le_bytes().to_vec();
        assert_eq!(
            residual(
                DataType::Float64,
                float.clone(),
                float,
                Scalar::from(1.5_f64)
            ),
            Some(Vec::new()),
            "a Float bound evolved to Double proves the matching value"
        );
    }

    #[test]
    fn malformed_bounds_leave_the_filter_for_rows() {
        assert_eq!(
            residual(DataType::Int64, vec![0; 3], vec![0; 9], Scalar::from(37)),
            Some(vec![0]),
            "malformed statistics cannot exclude or settle a file"
        );
    }
}
