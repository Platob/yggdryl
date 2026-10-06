//! `rust/src/iceberg/partition.rs`: what a partition document must spell.
//!
//! A spec read from someone else's catalog is only as safe as the reading is
//! strict, so these pin the identifiers, the one synthesizing rule v1 allows,
//! and the duplicates a spec refuses. All of it is `yggdryl::iceberg` API.

use yggdryl::iceberg::{PartitionField, PartitionSpec};
use yggdryl::{DataType, Field, Scalar, Serie};

#[test]
fn modern_partition_json_requires_exact_identifiers() {
    for text in [
        r#"{"name":"day","source-id":1,"transform":"identity"}"#,
        r#"{"name":"day","source-id":1,"field-id":2147483648,"transform":"identity"}"#,
    ] {
        let document = yggdryl::json::from_utf8(text).unwrap();
        assert!(PartitionField::from_json(&document).is_err(), "{text}");
    }
    for text in [
        r#"{"fields":[]}"#,
        r#"{"spec-id":2147483648,"fields":[]}"#,
        r#"{"spec-id":1,"fields":[{"name":"day","source-id":1,"transform":"identity"}]}"#,
    ] {
        let document = yggdryl::json::from_utf8(text).unwrap();
        assert!(PartitionSpec::from_json(&document).is_err(), "{text}");
    }
}

#[test]
fn only_the_v1_array_synthesizes_field_ids() {
    let document =
        yggdryl::json::from_utf8(r#"[{"name":"day","source-id":1,"transform":"identity"}]"#)
            .unwrap();
    let spec = PartitionSpec::from_json(&document).unwrap();
    assert_eq!(spec.spec_id, 0);
    assert_eq!(spec.fields[0].field_id, 1000);
}

/// A field array a caller hands over as a column reads as the run does, in
/// the v1 bare shape and under a v2 spec's `fields` alike.
#[test]
fn a_field_column_reads_as_the_array_it_holds() -> yggdryl::Result<()> {
    let fields = Scalar::from(Serie::empty(Field::new(
        "item",
        DataType::from_str("map<utf8, utf8>")?,
        false,
    ))?);
    assert!(PartitionSpec::from_json(&fields)?.is_unpartitioned());
    let document = yggdryl::json::from_utf8(r#"{"spec-id":0}"#)?.with_field("fields", fields)?;
    assert!(PartitionSpec::from_json(&document)?.is_unpartitioned());
    Ok(())
}

#[test]
fn a_spec_rejects_duplicate_field_ids_and_names() {
    for fields in [
        r#"[{"name":"a","source-id":1,"field-id":1000,"transform":"identity"},{"name":"b","source-id":2,"field-id":1000,"transform":"identity"}]"#,
        r#"[{"name":"a","source-id":1,"field-id":1000,"transform":"identity"},{"name":"a","source-id":2,"field-id":1001,"transform":"identity"}]"#,
    ] {
        let document =
            yggdryl::json::from_utf8(&format!(r#"{{"spec-id":1,"fields":{fields}}}"#)).unwrap();
        assert!(PartitionSpec::from_json(&document).is_err(), "{fields}");
    }
}

mod iceberg {
    use arrow_array::{Array, Int64Array, RecordBatch, StringArray};
    use std::sync::{Arc, Mutex};
    use yggdryl::arrow::BatchReader;
    use yggdryl::holder::Holder;
    use yggdryl::iceberg::{FormatVersion, IcebergTable, PartitionSpec, assign_field_ids};
    use yggdryl::local::LocalFolder;
    use yggdryl::media::IORecordOptions;
    use yggdryl::{DataType, Field, IOBase, IOMedia, Selector, StructType};

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
            pwrite, create_bytes, size, capacity, reserve, truncate, uri, url, bound_location, mtime, media_type,
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

    fn file_paths(table: &IcebergTable<impl IOBase>) -> Vec<String> {
        let mut paths: Vec<String> = table
            .data_files()
            .unwrap()
            .into_iter()
            .map(|(file, _)| file.file_path.to_string())
            .collect();
        paths.sort();
        paths
    }

    /// Three venues, three commits, three files.
    fn venues(label: &str) -> std::path::PathBuf {
        let path = root(label);
        let schema = schema();
        let spec = PartitionSpec::identity(1, &schema, &["venue"]).unwrap();
        let mut table = IcebergTable::create(
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
    fn partition_keys_are_the_primary_keys_of_a_merge() {
        let path = venues("primary-keys");
        let mut table = IcebergTable::open(Recording::new(&path)).unwrap();
        let before = file_paths(&table);

        // Keyed: (venue, id). Id 1 updates in XNAS, id 4 appends there, and no
        // other partition's file is opened or rewritten.
        table.root().reset();
        table
            .commit_merge(
                rows(&[1, 4], &["AAPL.O", "NVDA"], &["XNAS", "XNAS"]),
                &Selector::from_columns(["id"]),
                true,
            )
            .unwrap();
        let opened = table.root().data_files();
        assert_eq!(opened.len(), 1, "{opened:?}");
        assert!(opened[0].contains("venue=XNAS"));
        let after = file_paths(&table);
        assert_eq!(before.iter().filter(|p| after.contains(p)).count(), 2);

        // The same id in another partition is another row.
        table
            .commit_merge(
                rows(&[1], &["AAPL.L"], &["XLON"]),
                &Selector::from_columns(["id"]),
                true,
            )
            .unwrap();
        assert_eq!(
            triples(table.scan(None).unwrap()),
            vec![
                (1, "AAPL.L".to_owned(), "XLON".to_owned()),
                (1, "AAPL.O".to_owned(), "XNAS".to_owned()),
                (2, "MSFT".to_owned(), "XNYS".to_owned()),
                (3, "VOD".to_owned(), "XLON".to_owned()),
                (4, "NVDA".to_owned(), "XNAS".to_owned()),
            ]
        );

        // No key: the partition is the key, so XNYS is replaced whole, through
        // the record surface as much as through the commit method.
        let options = table.record_options().unwrap();
        assert!(options.merge_by().is_empty());
        table
            .merge_arrow_reader(rows(&[9], &["MSFT.N"], &["XNYS"]), &options)
            .unwrap();
        assert_eq!(
            triples(table.scan_where(&[("venue", "XNYS")], None).unwrap()),
            vec![(9, "MSFT.N".to_owned(), "XNYS".to_owned())]
        );
        assert_eq!(triples(table.scan(None).unwrap()).len(), 5);

        // Duplicate keys in one write keep the last row.
        table
            .commit_merge(
                rows(&[4, 4], &["first", "last"], &["XNAS", "XNAS"]),
                &Selector::from_columns(["id"]),
                true,
            )
            .unwrap();
        assert!(
            triples(table.scan_where(&[("venue", "XNAS")], None).unwrap()).contains(&(
                4,
                "last".to_owned(),
                "XNAS".to_owned()
            ))
        );
        let _ = std::fs::remove_dir_all(&path);
    }

    #[test]
    fn an_unpartitioned_table_still_needs_a_key_and_merges_as_before() {
        let path = root("flat-merge");
        let mut table = IcebergTable::create(
            LocalFolder::new(&path).unwrap(),
            FormatVersion::V2,
            schema(),
            PartitionSpec::unpartitioned(),
        )
        .unwrap();
        table
            .commit_append(rows(&[1, 2], &["a", "b"], &["X", "X"]))
            .unwrap();
        let refused = table
            .commit_merge(rows(&[3], &["c"], &["X"]), &Selector::all(), true)
            .unwrap_err()
            .to_string();
        assert!(refused.contains("match key"), "{refused}");
        table
            .commit_merge(
                rows(&[2, 3], &["b2", "c"], &["X", "X"]),
                &Selector::from_columns(["id"]),
                true,
            )
            .unwrap();
        assert_eq!(
            triples(table.scan(None).unwrap()),
            vec![
                (1, "a".to_owned(), "X".to_owned()),
                (2, "b2".to_owned(), "X".to_owned()),
                (3, "c".to_owned(), "X".to_owned()),
            ]
        );
        let _ = std::fs::remove_dir_all(&path);
    }
}

/// The time transforms: the specification's four and this crate's own
/// three, `minutes[n]`, `week` and `quarter`, each one name, its source types
/// and the grammar call that spells it.
mod time_transforms {
    use yggdryl::Term;
    use yggdryl::expression::Function;
    use yggdryl::iceberg::{PartitionField, PartitionSpec, Transform};
    use yggdryl::{DataType, TimeUnit, Timezone};

    const OWN: [(Transform, &str); 4] = [
        (Transform::Minutes(15), "minutes[15]"),
        (Transform::Minutes(30), "minutes[30]"),
        (Transform::Week, "week"),
        (Transform::Quarter, "quarter"),
    ];

    fn timestamp() -> DataType {
        DataType::DateTime64 {
            unit: TimeUnit::Microsecond,
            timezone: Timezone::NAIVE,
        }
    }

    #[test]
    fn each_spells_one_name() {
        for (transform, name) in OWN {
            assert_eq!(transform.to_string(), name);
            assert_eq!(Transform::from_str(name).unwrap(), transform);
            assert!(!transform.is_invertible(), "{name}");
        }
        assert_eq!(Transform::Minutes(1).to_string(), "minutes[1]");
        assert_eq!(
            Transform::from_str(" minutes[1] ").unwrap(),
            Transform::Minutes(1)
        );
        // Spark's DDL plurals are intake spellings of the parameter-free
        // time transforms - wide in, one canonical spelling out.
        for (plural, transform) in [
            ("years", Transform::Year),
            ("months", Transform::Month),
            ("days", Transform::Day),
            ("hours", Transform::Hour),
            ("weeks", Transform::Week),
            ("quarters", Transform::Quarter),
        ] {
            assert_eq!(Transform::from_str(plural).unwrap(), transform);
            assert_ne!(transform.to_string(), plural, "the singular is written");
        }
        let error = Transform::from_str("fortnight").unwrap_err().to_string();
        for name in ["minutes[n]", "week", "quarter"] {
            assert!(error.contains(name), "{error}");
        }
    }

    #[test]
    fn a_minutes_step_is_bracketed_positive_and_bounded() {
        // The one spelling is the bracketed one: no bare word, no other name,
        // no parenthesized step - which `bucket(n)` and `truncate(w)` keep -
        // and unsigned digits with nothing around them.
        for spelling in [
            "minutes[0]",
            "minutes",
            "minutes[x]",
            "minutes[15",
            "minutes[2147483646]",
            "minutes(15)",
            "minutes[-15]",
            "minutes[+15]",
            "minutes [15]",
            "minutes[ 15 ]",
            "minute",
            "min15",
            "qhour",
            "hhour",
        ] {
            let error = Transform::from_str(spelling).unwrap_err().to_string();
            assert!(error.contains("minutes[n]"), "{spelling}: {error}");
            assert!(error.contains(spelling), "{spelling}: {error}");
        }
        assert_eq!(
            Transform::from_str("bucket(16)").unwrap(),
            Transform::Bucket(16)
        );
        assert_eq!(
            Transform::from_str("truncate(4)").unwrap(),
            Transform::Truncate(4)
        );
        // The most minutes the official model can carry reads; a step past
        // it, or none, built by hand is refused by its spelling.
        assert_eq!(
            Transform::from_str("minutes[2147483645]").unwrap(),
            Transform::Minutes(2_147_483_645)
        );
        assert_eq!(
            Transform::Minutes(2_147_483_645)
                .result_type(&timestamp())
                .unwrap(),
            DataType::Int32
        );
        for step in [0, 2_147_483_646, u32::MAX] {
            let error = Transform::Minutes(step)
                .result_type(&timestamp())
                .unwrap_err()
                .to_string();
            assert!(error.contains(&format!("minutes[{step}]")), "{error}");
        }
    }

    #[test]
    fn result_types_are_int32_over_what_each_accepts() {
        for (transform, name) in OWN {
            assert_eq!(
                transform.result_type(&timestamp()).unwrap(),
                DataType::Int32,
                "{name}"
            );
        }
        for transform in [Transform::Week, Transform::Quarter] {
            assert_eq!(
                transform.result_type(&DataType::date32()).unwrap(),
                DataType::Int32
            );
        }
        for transform in [Transform::Minutes(15), Transform::Minutes(30)] {
            let error = transform
                .result_type(&DataType::date32())
                .unwrap_err()
                .to_string();
            assert!(error.contains("a timestamp"), "{error}");
            assert!(error.contains(&transform.to_string()), "{error}");
            assert!(error.contains("date32"), "{error}");
        }
        for (transform, name) in OWN {
            let error = transform
                .result_type(&DataType::Int64)
                .unwrap_err()
                .to_string();
            assert!(error.contains(name), "{error}");
            assert!(error.contains("int64"), "{error}");
        }
        let error = Transform::Week
            .result_type(&DataType::utf8())
            .unwrap_err()
            .to_string();
        assert!(error.contains("a date or a timestamp"), "{error}");
    }

    /// Every time transform reads a timestamp Iceberg can express - counted
    /// in microseconds or nanoseconds - and refuses one counted in seconds
    /// or milliseconds alike, the crate's own three by the same rule as the
    /// specification's four.
    #[test]
    fn every_time_transform_refuses_a_unit_iceberg_cannot_express() {
        for timezone in [Timezone::NAIVE, Timezone::UTC] {
            for transform in [
                Transform::Year,
                Transform::Month,
                Transform::Day,
                Transform::Hour,
                Transform::Minutes(60),
                Transform::Week,
                Transform::Quarter,
            ] {
                for unit in [TimeUnit::Microsecond, TimeUnit::Nanosecond] {
                    let source = DataType::DateTime64 { unit, timezone };
                    assert!(
                        transform.result_type(&source).is_ok(),
                        "{transform} over {source}"
                    );
                }
                for unit in [TimeUnit::Second, TimeUnit::Millisecond] {
                    let source = DataType::DateTime64 { unit, timezone };
                    let error = transform.result_type(&source).unwrap_err().to_string();
                    assert!(
                        error.contains(&source.to_string()),
                        "{transform} over {source}: {error}"
                    );
                }
            }
        }
    }

    #[test]
    fn the_seven_time_transforms_are_the_seven_epoch_functions() {
        for (transform, function) in [
            (Transform::Year, Function::Years),
            (Transform::Month, Function::Months),
            (Transform::Day, Function::Days),
            (Transform::Hour, Function::Hours),
            (Transform::Week, Function::Weeks),
            (Transform::Quarter, Function::Quarters),
        ] {
            assert_eq!(transform.function(), Some(function.clone()), "{transform}");
            assert_eq!(
                Transform::from_function(&function),
                Some(transform),
                "{function}"
            );
        }
        // `minutes` names its transform only with the step a call states.
        for step in [1, 15, 30] {
            assert_eq!(Transform::Minutes(step).function(), Some(Function::Minutes));
        }
        assert_eq!(Transform::from_function(&Function::Minutes), None);
        for transform in [
            Transform::Identity,
            Transform::Bucket(4),
            Transform::Truncate(2),
            Transform::Void,
            Transform::Unknown,
        ] {
            assert_eq!(transform.function(), None, "{transform}");
        }
        // A calendar part is a field of a date, never a period since the
        // epoch, so it spells no transform.
        for function in [
            Function::Year,
            Function::Month,
            Function::Day,
            Function::Hour,
            Function::Truncate,
            Function::Lower,
        ] {
            assert_eq!(Transform::from_function(&function), None, "{function}");
        }
    }

    #[test]
    fn a_call_and_a_transform_are_one_rule_both_ways() {
        let source = Term::column("ts");
        for (text, transform) in [
            ("years(ts)", Transform::Year),
            ("quarters(ts)", Transform::Quarter),
            ("months(ts)", Transform::Month),
            ("weeks(ts)", Transform::Week),
            ("days(ts)", Transform::Day),
            ("hours(ts)", Transform::Hour),
            ("minutes(ts, 1)", Transform::Minutes(1)),
            ("minutes(ts, 15)", Transform::Minutes(15)),
            ("minutes(ts, 30)", Transform::Minutes(30)),
        ] {
            let term: Term = text.parse().unwrap();
            assert_eq!(Transform::from_term(&term), Some(transform), "{text}");
            let spelled = transform.into_term(source.clone()).unwrap();
            assert_eq!(spelled, term, "{text}");
            assert_eq!(spelled.to_string(), text);
        }
        // A call with no step, a step that is not a positive whole literal,
        // the wrong arguments, and anything that is not an epoch call read
        // as no transform.
        let minutes = |arguments: Vec<Term>| Term::call(Function::Minutes, arguments);
        for term in [
            minutes(vec![Term::column("ts")]),
            minutes(vec![Term::column("ts"), Term::literal(0_i64)]),
            minutes(vec![Term::column("ts"), Term::literal("x")]),
            minutes(vec![Term::column("ts"), Term::column("n")]),
            minutes(vec![Term::column("ts"), Term::literal(-15_i64)]),
            Term::call(Function::Years, [Term::column("ts"), Term::literal(2_i64)]),
            "year(ts)".parse().unwrap(),
            "ts".parse().unwrap(),
        ] {
            assert_eq!(Transform::from_term(&term), None, "{term}");
        }
        for transform in [
            Transform::Identity,
            Transform::Bucket(16),
            Transform::Truncate(4),
            Transform::Void,
            Transform::Unknown,
            Transform::Minutes(0),
        ] {
            assert_eq!(transform.into_term(source.clone()), None, "{transform}");
        }
    }

    #[test]
    fn a_spec_of_the_crates_own_is_writable_and_round_trips_its_document() {
        let spec = PartitionSpec {
            spec_id: 3,
            fields: OWN
                .iter()
                .enumerate()
                .map(|(offset, (transform, _))| PartitionField {
                    source_id: 1,
                    field_id: 1000 + i32::try_from(offset).unwrap(),
                    name: format!("ts_{offset}").into(),
                    transform: *transform,
                })
                .collect(),
        };
        spec.require_writable().unwrap();
        let document = spec.clone().into_json().unwrap();
        let text = yggdryl::json::into_utf8(&document).unwrap();
        for (_, name) in OWN {
            assert!(text.contains(&format!("\"{name}\"")), "{text}");
        }
        assert_eq!(PartitionSpec::from_json(&document).unwrap(), spec);
    }
}

#[cfg(feature = "internals")]
mod internal {
    //! The partition value each time transform computes, reached through the
    //! write plan a commit resolves.

    use yggdryl::iceberg::{PartitionField, PartitionSpec, Transform, assign_field_ids};
    use yggdryl::internals::iceberg_partition::{PartitionTransform, write_transforms};
    use yggdryl::{DataType, Scalar, StructType, TimeUnit, Timezone};

    fn plan(transform: Transform, source: DataType) -> (PartitionSpec, PartitionTransform) {
        let mut schema = StructType::from_fields([source.required_field("at")])
            .map(DataType::from)
            .unwrap()
            .required_field("row");
        assign_field_ids(&mut schema, 1).unwrap();
        let spec = PartitionSpec {
            spec_id: 0,
            fields: vec![PartitionField {
                source_id: 1,
                field_id: 1000,
                // The field's name, never the transform's spelling, names
                // the directory: `at_minutes`, not `at_minutes[15]`.
                name: format!("at_{}", transform.to_string().split('[').next().unwrap()).into(),
                transform,
            }],
        };
        let partition = spec.partition_field(&schema).unwrap();
        let mut transforms = write_transforms(&spec, &schema, &partition).unwrap();
        (spec, transforms.remove(0))
    }

    fn micros(count: i64) -> Scalar {
        Scalar::datetime64(count, TimeUnit::Microsecond, Timezone::NAIVE).unwrap()
    }

    #[test]
    fn a_date_floors_to_its_week_and_quarter() {
        // 1969-12-28 is week -1, Monday 1969-12-29 week 0, 1970-01-04 the
        // last day of week 0, 1970-01-05 week 1.
        let (_, week) = plan(Transform::Week, DataType::date32());
        for (days, expected) in [(-4, -1), (-3, 0), (-1, 0), (0, 0), (3, 0), (4, 1)] {
            assert_eq!(
                week.partition_value(Scalar::date32(days)).unwrap(),
                Scalar::from(expected),
                "{days}"
            );
        }
        // 1970-03-31 is quarter 0, 1970-04-01 quarter 1, 1969-12-31 quarter -1.
        let (_, quarter) = plan(Transform::Quarter, DataType::date32());
        for (days, expected) in [(89, 0), (90, 1), (-1, -1), (0, 0)] {
            assert_eq!(
                quarter.partition_value(Scalar::date32(days)).unwrap(),
                Scalar::from(expected),
                "{days}"
            );
        }
        let (_, year) = plan(Transform::Year, DataType::date32());
        assert_eq!(
            year.partition_value(Scalar::date32(-1)).unwrap(),
            Scalar::from(-1)
        );
        assert_eq!(year.partition_value(Scalar::Null).unwrap(), Scalar::Null);
    }

    #[test]
    fn an_instant_floors_to_every_period() {
        // 2017-11-16T22:31:08, the instant Apache Iceberg's own fixtures use.
        let at = 1_510_871_468_000_000_i64;
        for (transform, count, expected) in [
            (Transform::Minutes(1), 59_999_999, Scalar::from(0)),
            (Transform::Minutes(1), 60_000_000, Scalar::from(1)),
            (Transform::Minutes(1), -1, Scalar::from(-1)),
            (Transform::Minutes(1), at, Scalar::from(25_181_191)),
            (Transform::Minutes(15), 899_999_999, Scalar::from(0)),
            (Transform::Minutes(15), 900_000_000, Scalar::from(1)),
            (Transform::Minutes(15), -1, Scalar::from(-1)),
            (Transform::Minutes(15), at, Scalar::from(1_678_746)),
            (Transform::Minutes(30), 1_799_999_999, Scalar::from(0)),
            (Transform::Minutes(30), 1_800_000_000, Scalar::from(1)),
            (Transform::Minutes(30), -1, Scalar::from(-1)),
            (Transform::Minutes(30), at, Scalar::from(839_373)),
            (Transform::Week, at, Scalar::from(2498)),
            (Transform::Week, -1, Scalar::from(0)),
            (Transform::Quarter, at, Scalar::from(191)),
            (Transform::Quarter, -1, Scalar::from(-1)),
            (Transform::Hour, at, Scalar::from(419_686)),
            (Transform::Hour, -1, Scalar::from(-1)),
            (Transform::Day, at, Scalar::date32(17_486)),
            (Transform::Day, -1, Scalar::date32(-1)),
            (Transform::Month, at, Scalar::from(574)),
            (Transform::Year, at, Scalar::from(47)),
            (Transform::Year, -1, Scalar::from(-1)),
        ] {
            let (_, plan) = plan(
                transform,
                DataType::DateTime64 {
                    unit: TimeUnit::Microsecond,
                    timezone: Timezone::NAIVE,
                },
            );
            assert_eq!(
                plan.partition_value(micros(count)).unwrap(),
                expected,
                "{transform} of {count}"
            );
        }
    }

    #[test]
    fn sixty_minutes_are_the_hour() {
        let timestamp = DataType::DateTime64 {
            unit: TimeUnit::Microsecond,
            timezone: Timezone::NAIVE,
        };
        let (_, minutes) = plan(Transform::Minutes(60), timestamp.clone());
        let (_, hour) = plan(Transform::Hour, timestamp);
        for count in [
            1_510_871_468_000_000_i64,
            0,
            -1,
            3_599_999_999,
            3_600_000_000,
            -3_600_000_001,
        ] {
            assert_eq!(
                minutes.partition_value(micros(count)).unwrap(),
                hour.partition_value(micros(count)).unwrap(),
                "{count}"
            );
        }
    }

    #[test]
    fn a_step_past_i64_in_nanoseconds_still_floors() {
        // The most minutes a table carries, counted in nanoseconds, is a
        // period no `i64` count reaches: every instant is in the period of
        // the epoch or the one before it.
        let (_, plan) = plan(
            Transform::Minutes(2_147_483_645),
            DataType::DateTime64 {
                unit: TimeUnit::Nanosecond,
                timezone: Timezone::NAIVE,
            },
        );
        let nanos =
            |count: i64| Scalar::datetime64(count, TimeUnit::Nanosecond, Timezone::NAIVE).unwrap();
        for (count, expected) in [(i64::MAX, 0), (0, 0), (-1, -1), (i64::MIN, -1)] {
            assert_eq!(
                plan.partition_value(nanos(count)).unwrap(),
                Scalar::from(expected),
                "{count}"
            );
        }
    }

    #[test]
    fn a_partition_path_spells_only_what_every_store_holds() {
        // An instant spells `:`, which no Windows path can: the directory
        // name keeps letters, digits and `._+-` and writes `_` for the rest,
        // the manifest staying the authority on the value.
        let mut schema = DataType::from(
            StructType::from_fields([DataType::DateTime64 {
                unit: TimeUnit::Nanosecond,
                timezone: Timezone::UTC,
            }
            .required_field("part")])
            .unwrap(),
        )
        .required_field("row");
        assign_field_ids(&mut schema, 1).unwrap();
        let spec = PartitionSpec::identity(1, &schema, &["part"]).unwrap();
        let value =
            Scalar::datetime64(900_000_000_000, TimeUnit::Nanosecond, Timezone::UTC).unwrap();
        let path = spec.partition_path(&[value]).unwrap();
        assert!(path.starts_with("part=1970-01-01T00_15_00"), "{path}");
        assert!(
            path.chars()
                .all(|character| character.is_ascii_alphanumeric()
                    || matches!(character, '.' | '_' | '+' | '-' | '=')),
            "{path}"
        );
        let venue = PartitionSpec::identity(
            1,
            &{
                let mut schema = DataType::from(
                    StructType::from_fields([DataType::utf8().required_field("venue")]).unwrap(),
                )
                .required_field("row");
                assign_field_ids(&mut schema, 1).unwrap();
                schema
            },
            &["venue"],
        )
        .unwrap();
        assert_eq!(
            venue.partition_path(&[Scalar::from("a/b c:d")]).unwrap(),
            "venue=a_b_c_d"
        );
    }

    #[test]
    fn a_partition_path_renders_the_period_number() {
        let (spec, plan) = plan(
            Transform::Minutes(15),
            DataType::DateTime64 {
                unit: TimeUnit::Microsecond,
                timezone: Timezone::NAIVE,
            },
        );
        let value = plan.partition_value(micros(1_510_871_468_000_000)).unwrap();
        assert_eq!(spec.partition_path(&[value]).unwrap(), "at_minutes=1678746");
        assert_eq!(
            spec.partition_path(&[Scalar::Null]).unwrap(),
            "at_minutes=null"
        );
    }
}

mod declared {
    //! A spec read from, and written as, a schema's `PARTITION:by`.

    use yggdryl::iceberg::{PartitionField, PartitionSpec, Transform, assign_field_ids};
    use yggdryl::{DataType, Field, StructType, TimeUnit, Timezone};

    fn schema() -> Field {
        let mut schema = DataType::from(
            StructType::from_fields([
                DataType::utf8().required_field("venue"),
                DataType::DateTime64 {
                    unit: TimeUnit::Microsecond,
                    timezone: Timezone::NAIVE,
                }
                .required_field("ts"),
                DataType::utf8().required_field("name"),
            ])
            .unwrap(),
        )
        .required_field("row");
        assign_field_ids(&mut schema, 1).unwrap();
        schema
    }

    #[test]
    fn a_declaration_reads_as_a_spec_and_a_spec_writes_its_declaration() {
        let declared = schema()
            .with_partition_by([
                "venue".parse().unwrap(),
                "minutes(ts, 15)".parse().unwrap(),
                "truncate(name, 4) as prefix".parse().unwrap(),
            ])
            .unwrap();
        let spec = PartitionSpec::from_schema(3, &declared).unwrap();
        assert_eq!(
            spec,
            PartitionSpec {
                spec_id: 3,
                fields: vec![
                    PartitionField::identity(1, 1000, "venue"),
                    PartitionField {
                        source_id: 2,
                        field_id: 1001,
                        name: "ts_minutes".into(),
                        transform: Transform::Minutes(15),
                    },
                    PartitionField {
                        source_id: 3,
                        field_id: 1002,
                        name: "prefix".into(),
                        transform: Transform::Truncate(4),
                    },
                ],
            }
        );

        // The spec marks the identity column and declares every field it
        // can spell, aliased where its name is not the convention's.
        let marked = spec.mark_partitions(&schema()).unwrap();
        assert_eq!(
            marked.partition_field_names().collect::<Vec<_>>(),
            ["venue"]
        );
        assert_eq!(
            marked.get_metadata("PARTITION:by"),
            Some(r#"["venue","minutes(ts, 15)","truncate(name, 4) as prefix"]"#)
        );
        assert_eq!(PartitionSpec::from_schema(3, &marked).unwrap(), spec);

        // A spec field named off the convention keeps its name as an alias.
        let renamed = PartitionSpec {
            spec_id: 1,
            fields: vec![
                PartitionField::identity(1, 1000, "v"),
                PartitionField {
                    source_id: 2,
                    field_id: 1001,
                    name: "ts_day".into(),
                    transform: Transform::Day,
                },
                PartitionField {
                    source_id: 2,
                    field_id: 1002,
                    name: "ts_bucket".into(),
                    transform: Transform::Bucket(8),
                },
            ],
        };
        let marked = renamed.mark_partitions(&schema()).unwrap();
        assert_eq!(
            marked.get_metadata("PARTITION:by"),
            Some(r#"["venue as v","days(ts)"]"#),
            "a bucket has no spelling and is left out of the declaration"
        );
        // Unpartitioned marks nothing and declares nothing.
        let plain = PartitionSpec::unpartitioned()
            .mark_partitions(&schema())
            .unwrap();
        assert_eq!(plain.get_metadata("PARTITION:by"), None);
        assert!(
            PartitionSpec::from_schema(0, &plain)
                .unwrap()
                .is_unpartitioned()
        );
    }

    #[test]
    fn a_derivation_no_transform_spells_partitions_on_the_column_it_materialized() {
        // `with_partition_by` makes each a `TRANSFORM:` column of the schema;
        // the spec partitions on that column by identity, which every engine
        // reads, and the table computes it for the rows it is written.
        for (entry, column) in [("lower(name)", "name_lower"), ("years(ts) + 1 as k", "k")] {
            let mut declared = schema()
                .with_partition_by([entry.parse().unwrap()])
                .unwrap();
            // The materialized column is numbered above the ones the schema has.
            assign_field_ids(&mut declared, 100).unwrap();
            let spec = PartitionSpec::from_schema(1, &declared).unwrap();
            assert_eq!(spec.fields.len(), 1, "{entry}");
            assert_eq!(spec.fields[0].transform, Transform::Identity, "{entry}");
            assert_eq!(spec.fields[0].name, column, "{entry}");
            let source = declared
                .fields()
                .iter()
                .find(|child| child.name() == column)
                .unwrap();
            assert_eq!(
                Some(spec.fields[0].source_id),
                source.parquet_field_id().unwrap(),
                "{entry}"
            );
        }
    }

    #[test]
    fn an_entry_no_spec_can_hold_is_refused_by_name() {
        // A transform spelled wrongly is refused whatever the schema holds.
        let declared = schema()
            .with_partition_by(["truncate(name, 0)".parse().unwrap()])
            .unwrap();
        let error = PartitionSpec::from_schema(1, &declared)
            .unwrap_err()
            .to_string();
        assert!(error.contains("Iceberg partition transform"), "{error}");
        assert!(error.contains("truncate(name, 0)"), "{error}");
        // A derivation declared on the root alone has no column to partition
        // on, and no spec field holds its term.
        for entry in ["lower(name)", "years(ts) + 1 as k"] {
            let mut declared = schema();
            declared.as_partition_mut().set_by_texts([entry]).unwrap();
            let error = PartitionSpec::from_schema(1, &declared)
                .unwrap_err()
                .to_string();
            assert!(
                error.contains("Iceberg partition transform"),
                "{entry}: {error}"
            );
            assert!(
                error.contains(entry.split(" as ").next().unwrap()),
                "{entry}: {error}"
            );
        }
        // A source with no field identifier is refused as it always was.
        let unnumbered = DataType::from(
            StructType::from_fields([DataType::utf8().required_field("venue")]).unwrap(),
        )
        .required_field("row")
        .with_partition_fields(&["venue"])
        .unwrap();
        let error = PartitionSpec::from_schema(1, &unnumbered)
            .unwrap_err()
            .to_string();
        assert!(error.contains("assign_field_ids"), "{error}");
    }
}

/// Every Iceberg primitive is the identity's own result type, through the
/// one door from the crate's vocabulary to the official one, and what that
/// door refuses - `unknown`, `variant`, a negative scale - is named.
#[test]
fn every_iceberg_primitive_is_an_identity_source_and_unknown_and_variant_are_none() {
    use yggdryl::iceberg::Transform;
    use yggdryl::{TimeUnit, Timezone};
    let sources = [
        DataType::Boolean,
        DataType::Int32,
        DataType::Int64,
        DataType::Float32,
        DataType::Float64,
        DataType::decimal128(10, 2).unwrap(),
        DataType::Decimal,
        DataType::Date32,
        DataType::time64(TimeUnit::Microsecond).unwrap(),
        DataType::datetime64(TimeUnit::Microsecond, Timezone::NAIVE).unwrap(),
        DataType::datetime64(TimeUnit::Microsecond, Timezone::UTC).unwrap(),
        DataType::datetime64(TimeUnit::Nanosecond, Timezone::NAIVE).unwrap(),
        DataType::datetime64(TimeUnit::Nanosecond, Timezone::UTC).unwrap(),
        DataType::utf8(),
        DataType::Ccy,
        DataType::Side,
        DataType::Uuid,
        DataType::fixed_binary(16).unwrap(),
        DataType::Binary,
    ];
    for source in sources {
        assert_eq!(
            Transform::Identity.result_type(&source).unwrap(),
            source,
            "{source}"
        );
    }
    assert_eq!(
        Transform::Bucket(16)
            .result_type(&DataType::decimal128(10, 2).unwrap())
            .unwrap(),
        DataType::Int32
    );
    for (source, named) in [
        (DataType::Null, "unknown"),
        (DataType::Variant, "variant"),
        (DataType::decimal64(10, -2).unwrap(), "-2"),
    ] {
        let refused = Transform::Identity
            .result_type(&source)
            .unwrap_err()
            .to_string();
        assert!(refused.contains(named), "{source}: {refused}");
    }
}
