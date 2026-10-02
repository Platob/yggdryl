//! `rust/src/iceberg/scan.rs`: the planning a scan does before it reads.
//!
//! A plan is what a caller receives, but every refusal and every pruning
//! decision inside it is a rule of its own - a delete manifest that must fail
//! before a file is opened, a partition tuple that must match its spec, a row
//! id that must inherit contiguously, a bound that must decode under the
//! schema it evolved to. Those steps are forwarded through
//! `yggdryl::internals`; what a caller observes of a planned scan is pinned in
//! `rust/tests/iceberg/mod_.rs`.

#[cfg(feature = "internals")]
mod internal {
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
                let error = validate_data_entry(
                    &entry(content),
                    &manifest,
                    &PartitionSpec::unpartitioned(),
                )
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
            residual_with_nans(dtype, lower, upper, value, Some(0))
        }

        fn residual_with_nans(
            dtype: DataType,
            lower: Vec<u8>,
            upper: Vec<u8>,
            value: Scalar,
            nans: Option<i64>,
        ) -> Option<Vec<usize>> {
            let schema = schema(dtype);
            let file = DataFile {
                record_count: 1,
                lower_bounds: vec![(1, lower)],
                upper_bounds: vec![(1, upper)],
                nan_value_counts: nans.map(|count| vec![(1, count)]).unwrap_or_default(),
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
                    float.clone(),
                    Scalar::from(1.5_f64)
                ),
                Some(Vec::new()),
                "a Float bound evolved to Double proves the matching value"
            );
            // Iceberg leaves NaN out of float bounds, so without a NaN count
            // of zero they neither prove nor rule out a row.
            for nans in [None, Some(1)] {
                assert_eq!(
                    residual_with_nans(
                        DataType::Float64,
                        float.clone(),
                        float.clone(),
                        Scalar::from(1.5_f64),
                        nans,
                    ),
                    Some(vec![0]),
                    "NaN count {nans:?}"
                );
                assert_eq!(
                    residual_with_nans(
                        DataType::Float64,
                        float.clone(),
                        float.clone(),
                        Scalar::from(9.5_f64),
                        nans,
                    ),
                    Some(vec![0]),
                    "NaN count {nans:?}"
                );
            }
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

    /// A time partition value is the period every row's source falls in, so
    /// it bounds the source column as a range and a predicate on the source
    /// prunes by it - a file by its tuple, a manifest by its summary.
    mod period_tests {
        use yggdryl::iceberg::{
            DataFile, FieldSummary, ManifestContent, ManifestFile, PartitionField, PartitionSpec,
            Transform,
        };
        use yggdryl::internals::iceberg_scan::{
            conjuncts, file_bounds, file_residual, identity_column, manifest_bounds,
            period_column_name,
        };
        use yggdryl::{DataType, Field, Filter, Scalar, StructType, TimeUnit, Timezone};

        /// `ts` (field 1), a required naive microsecond timestamp, and
        /// `day` (field 2), a nullable date.
        fn schema() -> Field {
            let mut schema = StructType::from_fields([
                DataType::DateTime64 {
                    unit: TimeUnit::Microsecond,
                    timezone: Timezone::NAIVE,
                }
                .required_field("ts"),
                DataType::date32().nullable_field("day"),
            ])
            .map(DataType::from)
            .unwrap()
            .required_field("row");
            yggdryl::iceberg::assign_field_ids(&mut schema, 1).unwrap();
            schema
        }

        /// `minutes[15]` of `ts` then `week` of `day`.
        fn spec() -> PartitionSpec {
            PartitionSpec {
                spec_id: 0,
                fields: vec![
                    PartitionField {
                        source_id: 1,
                        field_id: 1000,
                        name: "ts_minutes".into(),
                        transform: Transform::Minutes(15),
                    },
                    PartitionField {
                        source_id: 2,
                        field_id: 1001,
                        name: "day_week".into(),
                        transform: Transform::Week,
                    },
                ],
            }
        }

        fn residual(
            filter: &str,
            quarter_hour: Option<i32>,
            week: Option<i32>,
        ) -> Option<Vec<usize>> {
            let schema = schema();
            let file = DataFile {
                record_count: 4,
                partition: vec![
                    quarter_hour.map_or(Scalar::Null, Scalar::from),
                    week.map_or(Scalar::Null, Scalar::from),
                ],
                ..DataFile::default()
            };
            let filter: Filter = filter.parse().unwrap();
            let conjuncts = conjuncts(&schema, &filter).unwrap();
            file_residual(&file_bounds(&file, &spec(), &schema), &conjuncts)
        }

        #[test]
        fn a_file_is_bounded_by_the_period_its_tuple_names() {
            // Quarter hour 5 is 01:15 to 01:30 of 1970-01-01; a conjunct the
            // period settles is dropped, one it cannot stays for the rows,
            // and one it excludes excludes the file.
            assert_eq!(
                residual("ts >= '1970-01-01T01:15:00'", Some(5), Some(0)),
                Some(vec![])
            );
            assert_eq!(
                residual("ts <= '1970-01-01T01:29:59.999999'", Some(5), Some(0)),
                Some(vec![])
            );
            assert_eq!(
                residual("ts = '1970-01-01T01:20:00'", Some(5), Some(0)),
                Some(vec![0])
            );
            assert_eq!(
                residual("ts < '1970-01-01T01:15:00'", Some(5), Some(0)),
                None
            );
            assert_eq!(
                residual("ts > '1970-01-01T01:29:59.999999'", Some(5), Some(0)),
                None
            );
            assert_eq!(
                residual(
                    "ts between '1970-01-01T01:00:00' and '1970-01-01T01:14:59'",
                    Some(5),
                    Some(0)
                ),
                None
            );
            // Week 0 is Monday 1969-12-29 through Sunday 1970-01-04.
            assert_eq!(
                residual(
                    "day between '1969-12-29' and '1970-01-04'",
                    Some(5),
                    Some(0)
                ),
                Some(vec![])
            );
            assert_eq!(residual("day > '1970-01-04'", Some(5), Some(0)), None);
            assert_eq!(residual("day = '1969-12-28'", Some(5), Some(0)), None);
            assert_eq!(
                residual("day = '1970-01-01'", Some(5), Some(0)),
                Some(vec![0])
            );
            // A null period is a null source in every row.
            assert_eq!(residual("day is null", Some(5), None), Some(vec![]));
            assert_eq!(residual("day = '1970-01-01'", Some(5), None), None);
            assert_eq!(residual("day is not null", Some(5), Some(0)), Some(vec![]));
        }

        #[test]
        fn a_manifest_is_bounded_by_the_periods_its_summary_spans() {
            let manifest = ManifestFile {
                manifest_path: "metadata/periods.avro".into(),
                manifest_length: 128,
                partition_spec_id: 0,
                content: ManifestContent::Data,
                sequence_number: 1,
                min_sequence_number: 1,
                added_snapshot_id: 7,
                added_files_count: Some(3),
                existing_files_count: Some(0),
                deleted_files_count: Some(0),
                added_rows_count: Some(12),
                existing_rows_count: Some(0),
                deleted_rows_count: Some(0),
                partitions: vec![
                    // Quarter hours 5 through 7: 01:15 up to 02:00.
                    FieldSummary {
                        contains_null: false,
                        contains_nan: None,
                        lower_bound: Some(5_i32.to_le_bytes().to_vec()),
                        upper_bound: Some(7_i32.to_le_bytes().to_vec()),
                    },
                    FieldSummary {
                        contains_null: true,
                        contains_nan: None,
                        lower_bound: None,
                        upper_bound: None,
                    },
                ],
                key_metadata: None,
                first_row_id: None,
            };
            let schema = schema();
            let bounds = manifest_bounds(&manifest, &spec(), &schema);
            let prunes = |filter: &str| {
                let filter: Filter = filter.parse().unwrap();
                conjuncts(&schema, &filter)
                    .unwrap()
                    .iter()
                    .all(|conjunct| conjunct.statistics_prune(&bounds))
            };
            assert!(!prunes("ts < '1970-01-01T01:15:00'"));
            assert!(!prunes("ts >= '1970-01-01T02:00:00'"));
            assert!(prunes("ts >= '1970-01-01T01:59:59'"));
            assert!(prunes("ts = '1970-01-01T01:45:00'"));
            assert!(prunes("ts < '1970-01-01T01:15:00.000001'"));
            // A summary with no bounds and a null states nothing to prune by.
            assert!(prunes("day is null"));
            assert!(prunes("day = '1970-01-01'"));
        }

        #[test]
        fn a_period_column_is_its_source_and_an_identity_is_not_one() {
            let schema = schema();
            assert_eq!(
                period_column_name(&spec(), 0, &schema).as_deref(),
                Some("ts")
            );
            assert_eq!(
                period_column_name(&spec(), 1, &schema).as_deref(),
                Some("day")
            );
            assert!(identity_column(&spec(), 0, &schema).is_none());
            let mut bucketed = spec();
            bucketed.fields[0].transform = Transform::Bucket(8);
            bucketed.fields[1].transform = Transform::Identity;
            assert_eq!(period_column_name(&bucketed, 0, &schema), None);
            assert_eq!(period_column_name(&bucketed, 1, &schema), None);
            assert_eq!(
                identity_column(&bucketed, 1, &schema).map(Field::name),
                Some("day")
            );
        }
    }

    /// Every time transform bounds its source by exactly the period its value
    /// names, over every source it reads: the first and the last count of
    /// the period are kept, the count before it and the first of the next
    /// are excluded - by a file's tuple, and by a manifest's summary.
    mod period_range_tests {
        use yggdryl::iceberg::{
            DataFile, FieldSummary, ManifestContent, ManifestFile, PartitionField, PartitionSpec,
            Transform,
        };
        use yggdryl::internals::iceberg_scan::{
            conjuncts, file_bounds, file_residual, manifest_bounds,
        };
        use yggdryl::internals::timezone::days_from_civil;
        use yggdryl::{DataType, Field, Filter, Scalar, StructType, Term, TimeUnit, Timezone};

        const DAY: i64 = 86_400;

        /// One period of one transform: the number its value states, and the
        /// first second of it and of the next.
        struct Period {
            transform: Transform,
            number: i64,
            start: i64,
            next: i64,
        }

        fn day(year: i32, month: u32, day: u32) -> i64 {
            days_from_civil(year, month, day)
        }

        /// The periods 2024-05-17T13:47:23 falls in, and three before the
        /// epoch for the transforms of this crate's own, which no writer
        /// truncates.
        fn periods() -> Vec<Period> {
            let anchor = day(2024, 5, 17);
            let monday = day(2024, 5, 13);
            assert_eq!((monday + 3).rem_euclid(7), 0, "2024-05-13 is a Monday");
            let minute = anchor * 1_440 + 13 * 60 + 47;
            let calendar = |number: i64, from: (i32, u32, u32), to: (i32, u32, u32)| Period {
                transform: Transform::Unknown,
                number,
                start: day(from.0, from.1, from.2) * DAY,
                next: day(to.0, to.1, to.2) * DAY,
            };
            vec![
                Period {
                    transform: Transform::Year,
                    ..calendar(54, (2024, 1, 1), (2025, 1, 1))
                },
                Period {
                    transform: Transform::Quarter,
                    ..calendar(217, (2024, 4, 1), (2024, 7, 1))
                },
                Period {
                    transform: Transform::Month,
                    ..calendar(652, (2024, 5, 1), (2024, 6, 1))
                },
                Period {
                    transform: Transform::Week,
                    number: (monday + 3) / 7,
                    start: monday * DAY,
                    next: (monday + 7) * DAY,
                },
                Period {
                    transform: Transform::Day,
                    number: anchor,
                    start: anchor * DAY,
                    next: (anchor + 1) * DAY,
                },
                Period {
                    transform: Transform::Hour,
                    number: anchor * 24 + 13,
                    start: anchor * DAY + 13 * 3_600,
                    next: anchor * DAY + 14 * 3_600,
                },
                Period {
                    transform: Transform::Minutes(15),
                    number: minute / 15,
                    start: (minute / 15) * 900,
                    next: (minute / 15 + 1) * 900,
                },
                Period {
                    transform: Transform::Minutes(90),
                    number: minute / 90,
                    start: (minute / 90) * 5_400,
                    next: (minute / 90 + 1) * 5_400,
                },
                // Before the epoch: 1969-Q4, the week of Monday 1969-12-22,
                // and the quarter hour before midnight.
                Period {
                    transform: Transform::Quarter,
                    ..calendar(-1, (1969, 10, 1), (1970, 1, 1))
                },
                Period {
                    transform: Transform::Week,
                    number: -1,
                    start: -10 * DAY,
                    next: -3 * DAY,
                },
                Period {
                    transform: Transform::Minutes(15),
                    number: -1,
                    start: -900,
                    next: 0,
                },
            ]
        }

        fn schema(source: &DataType) -> Field {
            let mut schema = StructType::from_fields([source.clone().nullable_field("ts")])
                .map(DataType::from)
                .unwrap()
                .required_field("row");
            yggdryl::iceberg::assign_field_ids(&mut schema, 1).unwrap();
            schema
        }

        fn spec(transform: Transform) -> PartitionSpec {
            PartitionSpec {
                spec_id: 0,
                fields: vec![PartitionField {
                    source_id: 1,
                    field_id: 1000,
                    name: "ts_period".into(),
                    transform,
                }],
            }
        }

        fn value(period: &Period) -> Scalar {
            let number = i32::try_from(period.number).unwrap();
            match period.transform {
                Transform::Day => Scalar::date32(number),
                _ => Scalar::from(number),
            }
        }

        /// The source's own value `count` stands for: a day for a date, a
        /// count in the timestamp's unit.
        fn at(source: &DataType, count: i64) -> Scalar {
            match source {
                DataType::Date32 => Scalar::date32(i32::try_from(count).unwrap()),
                DataType::DateTime64 { unit, timezone } => {
                    Scalar::datetime64(count, *unit, *timezone).unwrap()
                }
                other => panic!("no period reads {other}"),
            }
        }

        fn residual(
            source: &DataType,
            period: &Period,
            value: Scalar,
            predicate: Term,
        ) -> Option<Vec<usize>> {
            let schema = schema(source);
            let file = DataFile {
                record_count: 4,
                partition: vec![value],
                ..DataFile::default()
            };
            let conjuncts = conjuncts(&schema, &Filter::new(predicate)).unwrap();
            file_residual(
                &file_bounds(&file, &spec(period.transform), &schema),
                &conjuncts,
            )
        }

        fn keeps_manifest(source: &DataType, period: &Period, predicate: Term) -> bool {
            let schema = schema(source);
            let bytes = i32::try_from(period.number).unwrap().to_le_bytes().to_vec();
            let manifest = ManifestFile {
                manifest_path: "metadata/periods.avro".into(),
                manifest_length: 128,
                partition_spec_id: 0,
                content: ManifestContent::Data,
                sequence_number: 1,
                min_sequence_number: 1,
                added_snapshot_id: 7,
                added_files_count: Some(1),
                existing_files_count: Some(0),
                deleted_files_count: Some(0),
                added_rows_count: Some(4),
                existing_rows_count: Some(0),
                deleted_rows_count: Some(0),
                partitions: vec![FieldSummary {
                    contains_null: false,
                    contains_nan: None,
                    lower_bound: Some(bytes.clone()),
                    upper_bound: Some(bytes),
                }],
                key_metadata: None,
                first_row_id: None,
            };
            let bounds = manifest_bounds(&manifest, &spec(period.transform), &schema);
            conjuncts(&schema, &Filter::new(predicate))
                .unwrap()
                .iter()
                .all(|conjunct| conjunct.statistics_prune(&bounds))
        }

        #[test]
        fn every_period_bounds_its_source_by_its_first_and_last_count() {
            let mut sources = vec![DataType::date32()];
            for unit in [
                TimeUnit::Second,
                TimeUnit::Millisecond,
                TimeUnit::Microsecond,
                TimeUnit::Nanosecond,
            ] {
                for timezone in [Timezone::NAIVE, Timezone::UTC] {
                    sources.push(DataType::DateTime64 { unit, timezone });
                }
            }
            let ts = || Term::column("ts");
            let mut checked = 0;
            for source in &sources {
                let per = match source {
                    DataType::DateTime64 { unit, .. } => match unit {
                        TimeUnit::Second => 1,
                        TimeUnit::Millisecond => 1_000,
                        TimeUnit::Microsecond => 1_000_000,
                        _ => 1_000_000_000,
                    },
                    _ => 0,
                };
                for period in periods() {
                    let (first, next) = if per == 0 {
                        // A date reads only the periods a day floors to.
                        if matches!(period.transform, Transform::Hour | Transform::Minutes(_)) {
                            continue;
                        }
                        (period.start / DAY, period.next / DAY)
                    } else {
                        (period.start * per, period.next * per)
                    };
                    let last = next - 1;
                    let before = first - 1;
                    let case = format!("{} {} over {source}", period.transform, period.number);
                    let lit = |count: i64| Term::literal(at(source, count));
                    let file =
                        |predicate: Term| residual(source, &period, value(&period), predicate);

                    assert!(file(ts().eq(lit(first))).is_some(), "{case}: first");
                    assert!(file(ts().eq(lit(last))).is_some(), "{case}: last");
                    assert_eq!(file(ts().eq(lit(before))), None, "{case}: before");
                    assert_eq!(file(ts().eq(lit(next))), None, "{case}: next");
                    assert_eq!(
                        file(ts().is_in([lit(before), lit(next)])),
                        None,
                        "{case}: in"
                    );
                    assert!(
                        file(ts().is_in([lit(before), lit(last)])).is_some(),
                        "{case}: in"
                    );
                    assert_eq!(file(ts().ge(lit(first)).not()), None, "{case}: not");
                    assert_eq!(file(ts().gt(lit(last)).not()), Some(vec![]), "{case}: not");
                    assert_eq!(
                        file(ts().between(lit(first), lit(last))),
                        Some(vec![]),
                        "{case}: between"
                    );
                    assert_eq!(file(ts().is_null()), None, "{case}: is null");
                    assert_eq!(
                        file(ts().is_not_null()),
                        Some(vec![]),
                        "{case}: is not null"
                    );
                    // A null period is a null source in every row.
                    assert_eq!(
                        residual(source, &period, Scalar::Null, ts().is_null()),
                        Some(vec![]),
                        "{case}: null"
                    );
                    assert_eq!(
                        residual(source, &period, Scalar::Null, ts().eq(lit(first))),
                        None,
                        "{case}: null"
                    );

                    // A table holds only what Iceberg expresses - a date, a
                    // timestamp in microseconds or nanoseconds - and a
                    // summary bounds its source by the same period; a day
                    // summary is the `date32` a `day` stores.
                    if matches!(per, 0 | 1_000_000 | 1_000_000_000) {
                        let keeps = |predicate: Term| keeps_manifest(source, &period, predicate);
                        assert!(keeps(ts().eq(lit(first))), "{case}: manifest first");
                        assert!(keeps(ts().eq(lit(last))), "{case}: manifest last");
                        assert!(!keeps(ts().eq(lit(before))), "{case}: manifest before");
                        assert!(!keeps(ts().eq(lit(next))), "{case}: manifest next");
                    }
                    checked += 1;
                }
            }
            // Seven periods a date floors to, eleven for each of the eight
            // timestamps.
            assert_eq!(checked, 7 + 8 * 11);
        }

        /// A writer that truncated a count before the epoch toward zero filed
        /// instants of period `k - 1` under `k` - iceberg-rust 0.10 still
        /// does for the last second of a day before 1970 - and Java's reader
        /// keeps such a file (`fixInclusiveTimeProjection`). So does a scan
        /// here: a period at or below zero of the specification's own
        /// transforms bounds its source from the start of the period before.
        #[test]
        fn a_truncating_writers_period_before_the_epoch_keeps_its_file() {
            use iceberg_official::spec::{Datum, Transform as OfficialTransform};
            use iceberg_official::transform::create_transform_function;

            // 1969-12-30T23:59:59.5 is day -2, and iceberg-rust files it
            // under day -1.
            let written = create_transform_function(&OfficialTransform::Day)
                .unwrap()
                .transform_literal(&Datum::timestamp_micros(-86_400_500_000))
                .unwrap();
            assert_eq!(written, Some(Datum::date(-1)));

            let micros = DataType::DateTime64 {
                unit: TimeUnit::Microsecond,
                timezone: Timezone::NAIVE,
            };
            let day = Period {
                transform: Transform::Day,
                number: -1,
                start: -DAY,
                next: 0,
            };
            let ts = || Term::column("ts");
            let lit = |micros_count: i64| Term::literal(at(&micros, micros_count));
            let kept = [
                ts().lt(lit(-86_400_000_000)),
                ts().eq(lit(-86_400_500_000)),
                ts().ge(lit(-2 * 86_400_000_000)),
            ];
            let excluded = [ts().lt(lit(-2 * 86_400_000_000)), ts().ge(lit(0))];
            for predicate in kept {
                assert!(
                    residual(&micros, &day, Scalar::date32(-1), predicate.clone()).is_some(),
                    "{predicate}"
                );
                assert!(
                    keeps_manifest(&micros, &day, predicate.clone()),
                    "{predicate}"
                );
            }
            for predicate in excluded {
                assert_eq!(
                    residual(&micros, &day, Scalar::date32(-1), predicate.clone()),
                    None,
                    "{predicate}"
                );
                assert!(
                    !keeps_manifest(&micros, &day, predicate.clone()),
                    "{predicate}"
                );
            }

            // Period zero is widened too, as Java widens the projection of
            // period -1 onto it: year 0 of a timestamp reaches back into 1969.
            let year = Period {
                transform: Transform::Year,
                number: 0,
                start: 0,
                next: day_count(1971) * DAY,
            };
            assert!(
                residual(&micros, &year, Scalar::from(0), ts().eq(lit(-1))).is_some(),
                "the last instant of 1969"
            );
            assert_eq!(
                residual(
                    &micros,
                    &year,
                    Scalar::from(0),
                    ts().lt(lit(-365 * 86_400_000_000))
                ),
                None,
                "before 1969"
            );

            // A date's year and month are widened; its day is the date itself,
            // which no writer truncates.
            let date = DataType::date32();
            let d = || Term::column("ts");
            let year_before = Period {
                transform: Transform::Year,
                number: -1,
                start: -365 * DAY,
                next: 0,
            };
            assert!(
                residual(
                    &date,
                    &year_before,
                    Scalar::from(-1),
                    d().eq(Term::literal(Scalar::date32(-366)))
                )
                .is_some(),
                "1968-12-31 under year -1"
            );
            assert_eq!(
                residual(
                    &date,
                    &year_before,
                    Scalar::from(-1),
                    d().lt(Term::literal(Scalar::date32(-365 - 366)))
                ),
                None,
                "before 1968"
            );
            let date_day = Period {
                transform: Transform::Day,
                number: -1,
                start: -DAY,
                next: 0,
            };
            assert_eq!(
                residual(
                    &date,
                    &date_day,
                    Scalar::date32(-1),
                    d().eq(Term::literal(Scalar::date32(-2)))
                ),
                None,
                "a date's day is exact"
            );
        }

        fn day_count(year: i32) -> i64 {
            days_from_civil(year, 1, 1)
        }
    }

    /// A column several partition fields read - an identity beside a period,
    /// or two periods - is bounded by the tightest of them, whatever order
    /// the spec lists them in.
    mod tightest_bounds_tests {
        use yggdryl::iceberg::{
            DataFile, FieldSummary, ManifestContent, ManifestFile, PartitionField, PartitionSpec,
            Transform,
        };
        use yggdryl::internals::iceberg_scan::{
            conjuncts, file_bounds, file_residual, manifest_bounds,
        };
        use yggdryl::{DataType, Field, Filter, Scalar, StructType, TimeUnit, Timezone};

        /// 2024-01-01 is day 19723.
        const NEW_YEAR: i64 = 19_723;

        fn schema() -> Field {
            let mut schema = StructType::from_fields([DataType::DateTime64 {
                unit: TimeUnit::Microsecond,
                timezone: Timezone::NAIVE,
            }
            .required_field("ts")])
            .map(DataType::from)
            .unwrap()
            .required_field("row");
            yggdryl::iceberg::assign_field_ids(&mut schema, 1).unwrap();
            schema
        }

        fn spec(transforms: &[Transform]) -> PartitionSpec {
            PartitionSpec {
                spec_id: 0,
                fields: transforms
                    .iter()
                    .enumerate()
                    .map(|(offset, transform)| PartitionField {
                        source_id: 1,
                        field_id: 1000 + i32::try_from(offset).unwrap(),
                        name: format!("ts_{offset}").into(),
                        transform: *transform,
                    })
                    .collect(),
            }
        }

        fn summary(lower: Vec<u8>, upper: Vec<u8>) -> FieldSummary {
            FieldSummary {
                contains_null: false,
                contains_nan: None,
                lower_bound: Some(lower),
                upper_bound: Some(upper),
            }
        }

        fn keeps(transforms: &[Transform], summaries: Vec<FieldSummary>, filter: &str) -> bool {
            let schema = schema();
            let manifest = ManifestFile {
                manifest_path: "metadata/tightest.avro".into(),
                manifest_length: 128,
                partition_spec_id: 0,
                content: ManifestContent::Data,
                sequence_number: 1,
                min_sequence_number: 1,
                added_snapshot_id: 7,
                added_files_count: Some(1),
                existing_files_count: Some(0),
                deleted_files_count: Some(0),
                added_rows_count: Some(4),
                existing_rows_count: Some(0),
                deleted_rows_count: Some(0),
                partitions: summaries,
                key_metadata: None,
                first_row_id: None,
            };
            let bounds = manifest_bounds(&manifest, &spec(transforms), &schema);
            let filter: Filter = filter.parse().unwrap();
            conjuncts(&schema, &filter)
                .unwrap()
                .iter()
                .all(|conjunct| conjunct.statistics_prune(&bounds))
        }

        #[test]
        fn an_identity_and_a_period_on_one_column_prune_by_the_tighter() {
            // Hours 0 to 3 of 2024-01-01, and the one instant 01:30 of it.
            let hours = summary(
                i32::try_from(NEW_YEAR * 24).unwrap().to_le_bytes().to_vec(),
                i32::try_from(NEW_YEAR * 24 + 3)
                    .unwrap()
                    .to_le_bytes()
                    .to_vec(),
            );
            let instant = (NEW_YEAR * 86_400 + 5_400) * 1_000_000;
            let identity = summary(
                instant.to_le_bytes().to_vec(),
                instant.to_le_bytes().to_vec(),
            );
            for (transforms, summaries) in [
                (
                    [Transform::Hour, Transform::Identity],
                    vec![hours.clone(), identity.clone()],
                ),
                (
                    [Transform::Identity, Transform::Hour],
                    vec![identity.clone(), hours.clone()],
                ),
            ] {
                assert!(
                    !keeps(&transforms, summaries.clone(), "ts = '2024-01-01T02:00:00'"),
                    "{transforms:?}"
                );
                assert!(
                    keeps(&transforms, summaries, "ts = '2024-01-01T01:30:00'"),
                    "{transforms:?}"
                );
            }
        }

        #[test]
        fn two_periods_on_one_column_prune_by_the_tighter_at_both_levels() {
            let day = summary(
                i32::try_from(NEW_YEAR).unwrap().to_le_bytes().to_vec(),
                i32::try_from(NEW_YEAR).unwrap().to_le_bytes().to_vec(),
            );
            let hour = summary(
                i32::try_from(NEW_YEAR * 24 + 1)
                    .unwrap()
                    .to_le_bytes()
                    .to_vec(),
                i32::try_from(NEW_YEAR * 24 + 1)
                    .unwrap()
                    .to_le_bytes()
                    .to_vec(),
            );
            let schema = schema();
            for (transforms, summaries, partition) in [
                (
                    [Transform::Day, Transform::Hour],
                    vec![day.clone(), hour.clone()],
                    vec![
                        Scalar::date32(i32::try_from(NEW_YEAR).unwrap()),
                        Scalar::from(i32::try_from(NEW_YEAR * 24 + 1).unwrap()),
                    ],
                ),
                (
                    [Transform::Hour, Transform::Day],
                    vec![hour.clone(), day.clone()],
                    vec![
                        Scalar::from(i32::try_from(NEW_YEAR * 24 + 1).unwrap()),
                        Scalar::date32(i32::try_from(NEW_YEAR).unwrap()),
                    ],
                ),
            ] {
                // The hour - 01:00 to 02:00 - is the tighter of the two.
                assert!(
                    !keeps(&transforms, summaries.clone(), "ts = '2024-01-01T05:00:00'"),
                    "{transforms:?}"
                );
                assert!(
                    keeps(&transforms, summaries, "ts = '2024-01-01T01:05:00'"),
                    "{transforms:?}"
                );
                let file = DataFile {
                    record_count: 4,
                    partition,
                    ..DataFile::default()
                };
                let residual = |filter: &str| {
                    let filter: Filter = filter.parse().unwrap();
                    file_residual(
                        &file_bounds(&file, &spec(&transforms), &schema),
                        &conjuncts(&schema, &filter).unwrap(),
                    )
                };
                assert_eq!(
                    residual("ts = '2024-01-01T05:00:00'"),
                    None,
                    "{transforms:?}"
                );
                assert_eq!(
                    residual("ts between '2024-01-01T01:00:00' and '2024-01-01T01:59:59.999999'"),
                    Some(vec![]),
                    "{transforms:?}"
                );
            }
        }
    }

    /// A data file's tuple prunes it when its entry carries no column
    /// statistics at all - the case a foreign writer's manifest is, and the
    /// one a table this crate writes never shows, because its writer always
    /// records the column's bounds.
    mod statistics_free_tests {
        use yggdryl::holder::{Buffer, Holder};
        use yggdryl::iceberg::{
            DataFile, FormatVersion, ManifestContent, ManifestEntry, ManifestFile, PartitionField,
            PartitionSpec, Transform, write_manifest,
        };
        use yggdryl::internals::iceberg_scan::{conjuncts, plan};
        use yggdryl::{DataType, Filter, MimeType, Scalar, StructType, TimeUnit, Timezone};

        #[test]
        fn a_manifest_without_column_statistics_still_prunes_files_by_their_period() {
            let mut schema = StructType::from_fields([DataType::DateTime64 {
                unit: TimeUnit::Microsecond,
                timezone: Timezone::NAIVE,
            }
            .required_field("ts")])
            .map(DataType::from)
            .unwrap()
            .required_field("row");
            yggdryl::iceberg::assign_field_ids(&mut schema, 1).unwrap();
            let spec = PartitionSpec {
                spec_id: 0,
                fields: vec![PartitionField {
                    source_id: 1,
                    field_id: 1000,
                    name: "ts_minutes".into(),
                    transform: Transform::Minutes(15),
                }],
            };
            // Quarter hours 0, 1, 2 and 4 of 1970-01-01, one file each, and
            // not one column statistic among them.
            let entries: Vec<ManifestEntry> = [0, 1, 2, 4]
                .into_iter()
                .map(|quarter_hour: i32| {
                    ManifestEntry::added(
                        7,
                        DataFile {
                            file_path: format!("data/ts_minutes={quarter_hour}/part.parquet")
                                .into(),
                            mime_type: MimeType::PARQUET,
                            record_count: 2,
                            file_size_in_bytes: 128,
                            partition: vec![Scalar::from(quarter_hour)],
                            ..DataFile::default()
                        },
                    )
                })
                .collect();
            let mut handle = Buffer::new();
            write_manifest(&mut handle, FormatVersion::V2, &schema, &spec, &entries).unwrap();
            let manifest = ManifestFile {
                manifest_path: "metadata/statistics-free.avro".into(),
                manifest_length: 128,
                partition_spec_id: 0,
                content: ManifestContent::Data,
                sequence_number: 1,
                min_sequence_number: 1,
                added_snapshot_id: 7,
                added_files_count: Some(4),
                existing_files_count: Some(0),
                deleted_files_count: Some(0),
                added_rows_count: Some(8),
                existing_rows_count: Some(0),
                deleted_rows_count: Some(0),
                // No summary either, so the manifest is read and its files
                // are pruned by their tuples alone.
                partitions: Vec::new(),
                key_metadata: None,
                first_row_id: None,
            };
            let planned = |filter: &str| {
                let filter: Filter = filter.parse().unwrap();
                plan(
                    std::slice::from_ref(&manifest),
                    &|_| Ok(spec.clone()),
                    &|_| Ok(Holder::from(handle.clone())),
                    &conjuncts(&schema, &filter).unwrap(),
                    &schema,
                    true,
                )
                .unwrap()
            };

            let early = planned("ts < '1970-01-01T00:15:00'");
            assert_eq!(early.manifests_read, 1);
            assert_eq!(early.files_skipped(), 3);
            assert_eq!(early.tasks.len(), 1);
            assert_eq!(early.tasks[0].data_file().partition, vec![Scalar::from(0)]);
            assert!(early.tasks[0].residual.is_empty(), "the period settles it");
            for task in &early.excluded {
                assert!(
                    task.data_file().lower_bounds.is_empty()
                        && task.data_file().upper_bounds.is_empty(),
                    "no column statistic decided it"
                );
            }

            let late = planned("ts >= '1970-01-01T00:30:00'");
            assert_eq!(late.files_skipped(), 2);
            assert_eq!(
                late.tasks
                    .iter()
                    .map(|task| task.data_file().partition.clone())
                    .collect::<Vec<_>>(),
                vec![vec![Scalar::from(2)], vec![Scalar::from(4)]]
            );
            let within = planned("ts = '1970-01-01T00:20:00'");
            assert_eq!(within.files_skipped(), 3);
            assert_eq!(
                within.tasks[0].residual,
                vec![0],
                "the rows still answer it"
            );
        }
    }
}

mod iceberg {
    use arrow_array::{Array, Int64Array, RecordBatch, StringArray};
    use std::sync::{Arc, Mutex};
    use yggdryl::arrow::BatchReader;
    use yggdryl::holder::Holder;
    use yggdryl::iceberg::{FormatVersion, IcebergOptions, PartitionSpec, Table, assign_field_ids};
    use yggdryl::local::LocalFolder;
    use yggdryl::media::{IORecordOptions, RecordOptions};
    use yggdryl::{DataType, Field, IOBase, IOMedia, StructType};

    /// A table folder that records every relative path resolved through it.
    ///
    /// Every data file a scan opens is one `child_by_path` on the table's root,
    /// so the paths recorded here are the files a read or a merge touched.
    #[derive(Debug)]
    struct Recording {
        inner: LocalFolder,
        seen: Arc<Mutex<Vec<String>>>,
    }

    impl Recording {
        fn new(path: &std::path::Path) -> Self {
            Self {
                inner: LocalFolder::new(path).unwrap(),
                seen: Arc::new(Mutex::new(Vec::new())),
            }
        }

        fn data_files(&self) -> Vec<String> {
            let mut seen: Vec<String> = self
                .seen
                .lock()
                .unwrap()
                .iter()
                .filter(|path| path.starts_with("data/"))
                .cloned()
                .collect();
            seen.sort();
            seen.dedup();
            seen
        }

        fn reset(&self) {
            self.seen.lock().unwrap().clear();
        }
    }

    impl IOMedia for Recording {
        yggdryl::impl_default_iomedia!();
    }

    impl IOBase for Recording {
        yggdryl::delegate_iobase!(inner: pread, read_all_bytes, read_range_bytes, pstream_bytes,
            pwrite, size, capacity, reserve, truncate, uri, url, bound_location, mtime, media_type,
            set_media_type, flush, open, opened, close, parent, ls, kind, clear, remove,
            is_atomic, is_tabular, is_io);

        fn child_by_path(&self, path: &str) -> yggdryl::Result<Holder> {
            self.seen.lock().unwrap().push(path.to_owned());
            self.inner.child_by_path(path)
        }
    }

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

    fn schema() -> Field {
        let mut schema = StructType::from_fields([
            DataType::Int64.required_field("id"),
            DataType::utf8().nullable_field("symbol"),
            DataType::utf8().nullable_field("venue"),
        ])
        .map(DataType::from)
        .unwrap()
        .required_field("row");
        assign_field_ids(&mut schema, 1).unwrap();
        schema
    }

    fn rows(ids: &[i64], symbols: &[&str], venues: &[&str]) -> BatchReader {
        let batch = RecordBatch::try_new(
            schema().into_arrow_schema().unwrap(),
            vec![
                Arc::new(Int64Array::from(ids.to_vec())),
                Arc::new(StringArray::from(symbols.to_vec())),
                Arc::new(StringArray::from(venues.to_vec())),
            ],
        )
        .unwrap();
        yggdryl::arrow::batch_reader(batch.schema(), [batch])
    }

    /// Every `(id, symbol, venue)` a reader yields, sorted.
    fn triples(reader: BatchReader) -> Vec<(i64, String, String)> {
        let mut out = Vec::new();
        for batch in reader {
            let batch = batch.unwrap();
            let ids = batch
                .column_by_name("id")
                .unwrap()
                .as_any()
                .downcast_ref::<Int64Array>()
                .unwrap();
            let symbols = batch
                .column_by_name("symbol")
                .unwrap()
                .as_any()
                .downcast_ref::<StringArray>()
                .unwrap();
            let venues = batch
                .column_by_name("venue")
                .unwrap()
                .as_any()
                .downcast_ref::<StringArray>()
                .unwrap();
            for row in 0..batch.num_rows() {
                out.push((
                    ids.value(row),
                    symbols.value(row).to_owned(),
                    venues.value(row).to_owned(),
                ));
            }
        }
        out.sort();
        out
    }

    /// Three venues, three commits, three files.
    fn venues(label: &str) -> std::path::PathBuf {
        let path = root(label);
        let schema = schema();
        let spec = PartitionSpec::identity(1, &schema, &["venue"]).unwrap();
        let mut table = Table::create(
            LocalFolder::new(&path).unwrap(),
            FormatVersion::V2,
            schema,
            spec,
        )
        .unwrap();
        for (id, symbol, venue) in [
            (1_i64, "AAPL", "XNAS"),
            (2, "MSFT", "XNYS"),
            (3, "VOD", "XLON"),
        ] {
            table
                .commit_append(rows(&[id], &[symbol], &[venue]))
                .unwrap();
        }
        path
    }

    #[test]
    fn a_where_on_a_record_read_prunes_with_the_whole_expression_language() {
        let path = venues("where-pushdown");
        let table = Table::open(Recording::new(&path)).unwrap();

        // The plan is what a read runs: a membership and a range skip manifests
        // exactly as an equality does.
        assert_eq!(
            table
                .plan_matching("venue = 'XNAS'")
                .unwrap()
                .manifests_skipped(),
            2
        );
        assert_eq!(
            table
                .plan_matching("venue in ('XNAS', 'XLON')")
                .unwrap()
                .manifests_skipped(),
            1
        );
        assert_eq!(
            table
                .plan_matching("venue between 'XLON' and 'XNAS'")
                .unwrap()
                .manifests_skipped(),
            1
        );

        // A read through the record options opens only the surviving files.
        let options: RecordOptions = table
            .record_options()
            .unwrap()
            .with_filter("venue in ('XNAS', 'XLON') and id > 0")
            .unwrap();
        table.root().reset();
        let read = triples(table.read_arrow_reader(&options).unwrap());
        assert_eq!(
            read,
            vec![
                (1, "AAPL".to_owned(), "XNAS".to_owned()),
                (3, "VOD".to_owned(), "XLON".to_owned()),
            ]
        );
        let opened = table.root().data_files();
        assert_eq!(opened.len(), 2, "{opened:?}");
        assert!(opened.iter().all(|file| !file.contains("venue=XNYS")));

        // The folder route pushes the same clause down.
        let folder = LocalFolder::new(&path).unwrap();
        let read = triples(folder.read_arrow_reader(&options).unwrap());
        assert_eq!(read.len(), 2);
        let _ = std::fs::remove_dir_all(&path);
    }

    /// Every `(id, symbol, quantity)` a reader yields, in the order it yields
    /// them.
    fn quantities(reader: BatchReader) -> Vec<(i64, String, Option<i64>)> {
        let mut out = Vec::new();
        for batch in reader {
            let batch = batch.unwrap();
            let ids = batch
                .column_by_name("id")
                .unwrap()
                .as_any()
                .downcast_ref::<Int64Array>()
                .unwrap();
            let symbols = batch
                .column_by_name("symbol")
                .unwrap()
                .as_any()
                .downcast_ref::<StringArray>()
                .unwrap();
            let quantities = batch
                .column_by_name("quantity")
                .unwrap()
                .as_any()
                .downcast_ref::<Int64Array>()
                .unwrap();
            for row in 0..batch.num_rows() {
                out.push((
                    ids.value(row),
                    symbols.value(row).to_owned(),
                    quantities.is_valid(row).then(|| quantities.value(row)),
                ));
            }
        }
        out
    }

    #[test]
    fn files_decoding_to_different_layouts_each_cast_and_project_in_one_scan() {
        let path = root("scan_layouts");
        let mut table = Table::create(
            LocalFolder::new(&path).unwrap(),
            FormatVersion::V2,
            schema(),
            PartitionSpec::unpartitioned(),
        )
        .unwrap();
        table
            .commit_append(rows(&[1, 2], &["AAPL", "MSFT"], &["XNAS", "XNYS"]))
            .unwrap();

        // The first file stores three columns; every later one stores four.
        let mut evolved = table.schema().unwrap().clone();
        evolved.remove_metadata("ICEBERG:schema-id");
        let mut fields = evolved.fields().to_vec();
        fields.push(DataType::Int64.nullable_field("quantity"));
        evolved
            .set_dtype(DataType::from(StructType::from_fields(fields).unwrap()))
            .unwrap();
        assign_field_ids(&mut evolved, 4).unwrap();
        table.evolve_schema(evolved).unwrap();
        let wide = table.schema().unwrap().clone().into_arrow_schema().unwrap();
        for (ids, symbols, venues, quantities) in [
            ([3_i64, 4], ["VOD", "BP"], ["XLON", "XNYS"], [7_i64, 8]),
            ([5, 6], ["SAP", "SIE"], ["XETR", "XNYS"], [9, 10]),
        ] {
            let batch = RecordBatch::try_new(
                Arc::clone(&wide),
                vec![
                    Arc::new(Int64Array::from(ids.to_vec())),
                    Arc::new(StringArray::from(symbols.to_vec())),
                    Arc::new(StringArray::from(venues.to_vec())),
                    Arc::new(Int64Array::from(quantities.to_vec())),
                ],
            )
            .unwrap();
            table
                .commit_append(yggdryl::arrow::batch_reader(Arc::clone(&wide), [batch]))
                .unwrap();
        }

        // Without a projection each file comes back in its own layout, so the
        // cast into the read root changes plan between files; with one that
        // leaves out the filtered column, a second cast drops it again.
        let projection = table
            .schema()
            .unwrap()
            .clone()
            .without_fields(&["venue"])
            .unwrap();
        let expected = |mut rows: Vec<(i64, String, Option<i64>)>| {
            rows.sort();
            rows
        };
        let want = vec![
            (1, "AAPL".to_owned(), None),
            (3, "VOD".to_owned(), Some(7)),
            (5, "SAP".to_owned(), Some(9)),
        ];
        for options in [
            IcebergOptions::new().try_with_read_parallelism(1).unwrap(),
            IcebergOptions::new()
                .try_with_read_parallelism(2)
                .unwrap()
                .with_read_parallel_min_files(1)
                .with_read_parallel_min_file_size_bytes(0),
        ] {
            table.set_options(options);
            for field in [None, Some(&projection)] {
                let read = quantities(table.scan_matching("venue <> 'XNYS'", field).unwrap());
                assert_eq!(expected(read), want, "projection: {}", field.is_some());
            }
        }
        let _ = std::fs::remove_dir_all(&path);
    }
}
