//! `rust/src/iceberg/table.rs`: what a commit decides on the way through.
//!
//! A caller sees one call succeed or conflict, so the decisions inside it are
//! pinned here one at a time - the retry ladder's budget and jitter, the merge
//! pruning a malformed or NaN bound must never win, the partition summary a
//! NaN must not enter, the metadata names a listing accepts, and the relative
//! location two writers spell differently. All of it comes through
//! `yggdryl::internals`; what a caller observes of a committed table is pinned
//! in `rust/tests/iceberg/mod_.rs`.

mod retry_tests {
    use yggdryl::internals::iceberg_table::{
        CommitSettings, backoff_ms, reserve_retry_backoff, retry_wait_ms,
    };

    #[test]
    fn retry_budget_includes_its_exact_boundary_and_exhaustion_is_a_conflict() {
        let mut spent = 4;
        assert!(reserve_retry_backoff(&mut spent, 1, 5));
        assert_eq!(spent, 5);
        assert!(!reserve_retry_backoff(&mut spent, 1, 5));
        assert_eq!(spent, 5, "a refused wait does not consume budget");

        let settings = CommitSettings::new(1, 0, 0, 0);
        let mut beaten = 0;
        let mut spent = 0;
        assert_eq!(
            retry_wait_ms(&settings, &mut beaten, &mut spent, 2, 2).unwrap(),
            0
        );
        let error = retry_wait_ms(&settings, &mut beaten, &mut spent, 2, 2).unwrap_err();
        assert!(error.is_conflict(), "{error}");
    }

    #[test]
    fn full_jitter_never_exceeds_its_exponential_window() {
        for attempt in 0..8 {
            let cap = 10_u64.saturating_mul(1_u64 << attempt).min(100);
            for _ in 0..32 {
                assert!(backoff_ms(attempt, 10, 100) <= cap);
            }
        }
        assert_eq!(backoff_ms(4, 0, 100), 0);
    }
}

mod key_bound_tests {
    use std::sync::Arc;

    use arrow_array::{Float64Array, RecordBatch};

    use yggdryl::iceberg::{DataFile, ManifestEntry, PartitionSpec};
    use yggdryl::internals::iceberg_table::{KeyBound, KeyBounds, summaries};
    use yggdryl::internals::iceberg_value::single_value;
    use yggdryl::{DataType, Scalar, Selector, StructType};

    #[test]
    fn malformed_external_bounds_cannot_exclude_a_merge_file() {
        let incoming = 37_i64.to_le_bytes().to_vec();
        let bound = KeyBound::new(
            1,
            DataType::Int64,
            false,
            false,
            Some(incoming.clone()),
            Some(incoming),
        );
        let file = DataFile {
            record_count: 1,
            lower_bounds: vec![(1, vec![0; 3])],
            upper_bounds: vec![(1, vec![0; 9])],
            null_value_counts: vec![(1, 0)],
            ..DataFile::default()
        };
        assert!(bound.may_hold(&file));

        let incoming = 1.5_f64.to_le_bytes().to_vec();
        let float_bound = KeyBound::new(
            1,
            DataType::Float64,
            false,
            false,
            Some(incoming.clone()),
            Some(incoming),
        );
        let nan = f64::NAN.to_le_bytes().to_vec();
        let file = DataFile {
            record_count: 1,
            lower_bounds: vec![(1, nan.clone())],
            upper_bounds: vec![(1, nan)],
            null_value_counts: vec![(1, 0)],
            ..DataFile::default()
        };
        assert!(float_bound.may_hold(&file));
    }

    #[test]
    fn generated_nan_merge_bounds_are_conservatively_unbounded() {
        let mut schema = StructType::from_fields([DataType::Float64.required_field("ratio")])
            .map(DataType::from)
            .unwrap()
            .required_field("row");
        yggdryl::iceberg::assign_field_ids(&mut schema, 1).unwrap();
        let arrow_schema = schema.clone().into_arrow_schema().unwrap();
        let batch = RecordBatch::try_new(
            arrow_schema,
            vec![Arc::new(Float64Array::from(vec![1.5, f64::NAN]))],
        )
        .unwrap();

        let bounds = KeyBounds::of(&[batch], &schema, &Selector::from_columns(["ratio"])).unwrap();
        assert!(bounds.column_is_unbounded(0));
    }

    #[test]
    fn partition_summaries_omit_nan_bounds() {
        let mut schema = StructType::from_fields([DataType::Float64.required_field("ratio")])
            .map(DataType::from)
            .unwrap()
            .required_field("row");
        yggdryl::iceberg::assign_field_ids(&mut schema, 1).unwrap();
        let spec = PartitionSpec::identity(0, &schema, &["ratio"]).unwrap();
        let entry = |value| {
            ManifestEntry::added(
                1,
                DataFile {
                    record_count: 1,
                    partition: vec![value],
                    ..DataFile::default()
                },
            )
        };

        let only_nan = summaries(&spec, &schema, &[entry(Scalar::from(f64::NAN))]).unwrap();
        assert!(only_nan[0].lower_bound.is_none());
        assert!(only_nan[0].upper_bound.is_none());

        let finite = Scalar::from(1.5_f64);
        let encoded = single_value(&finite, &DataType::Float64).unwrap();
        let mixed = summaries(
            &spec,
            &schema,
            &[entry(Scalar::from(f64::NAN)), entry(finite)],
        )
        .unwrap();
        assert_eq!(mixed[0].lower_bound.as_deref(), Some(encoded.as_slice()));
        assert_eq!(mixed[0].upper_bound.as_deref(), Some(encoded.as_slice()));
    }
}

mod metadata_name_tests {
    use yggdryl::internals::iceberg_table::metadata_version_from_name;

    #[test]
    fn accepts_exact_hadoop_and_official_metadata_names() {
        for (name, version) in [
            ("v3.metadata.json", 3),
            ("v00003.gz.metadata.json", 3),
            (
                "00003-2cd22b57-5127-4198-92ba-e4e67c79821b.metadata.json",
                3,
            ),
            ("9-2cd22b57-5127-4198-92ba-e4e67c79821b.gz.metadata.json", 9),
        ] {
            assert_eq!(metadata_version_from_name(name), Some(version), "{name}");
        }
    }

    #[test]
    fn rejects_metadata_lookalikes() {
        for name in [
            "v.metadata.json",
            "v2backup.metadata.json",
            "vv2.metadata.json",
            "00002-not-a-uuid.metadata.json",
            "00002-2cd22b57-5127-4198-92ba-e4e67c79821b.extra.metadata.json",
            "00002-2cd22b57-5127-4198-92ba-e4e67c79821b.zst.metadata.json",
            "2.metadata.json",
            "v2.json",
        ] {
            assert_eq!(metadata_version_from_name(name), None, "{name}");
        }
    }
}

mod location_tests {
    use yggdryl::internals::iceberg_table::relative_location;

    #[test]
    fn an_empty_uri_authority_is_the_same_place_spelled_shorter() {
        // Whichever writer spelled which: the crate writes the table location
        // with the authority, Spark commits its manifest lists without it.
        for (base, location) in [
            (
                "file:///warehouse/db/t",
                "file:/warehouse/db/t/metadata/snap-1.avro",
            ),
            (
                "file:/warehouse/db/t",
                "file:///warehouse/db/t/metadata/snap-1.avro",
            ),
            (
                "file:/warehouse/db/t",
                "file:/warehouse/db/t/metadata/snap-1.avro",
            ),
        ] {
            assert_eq!(
                relative_location(base, location).unwrap(),
                "metadata/snap-1.avro",
                "{base} -> {location}"
            );
        }
    }

    #[test]
    fn a_real_authority_and_a_windows_drive_are_left_alone() {
        assert_eq!(
            relative_location("s3://bucket/db/t", "s3://bucket/db/t/data/0.parquet").unwrap(),
            "data/0.parquet"
        );
        assert_eq!(
            relative_location("C:\\warehouse\\t", "C:\\warehouse\\t\\data\\0.parquet").unwrap(),
            "data/0.parquet"
        );
        // A neighbour is not a child, however either side is spelled.
        relative_location("file:///warehouse/db/t", "file:/warehouse/db/other/x").unwrap_err();
        relative_location("s3://bucket/db/t", "s3://other/db/t/x").unwrap_err();
    }
}
