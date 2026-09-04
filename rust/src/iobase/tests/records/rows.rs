use super::*;

#[test]
fn native_struct_row_adapters_route_all_three_intents() {
    let mut handle = handle("native-row-adapters.arrows");
    let options = handle
        .record_options()
        .unwrap()
        .with_field(schema())
        .with_batch_row_size(1);

    handle
        .overwrite_records(
            [
                NativeRow {
                    id: 1,
                    symbol: Some("AAPL"),
                },
                NativeRow {
                    id: 2,
                    symbol: None,
                },
            ],
            &options,
        )
        .unwrap();
    handle
        .append_records(
            [NativeRow {
                id: 3,
                symbol: Some("MSFT"),
            }],
            &options,
        )
        .unwrap();
    handle
        .merge_records(
            [
                NativeRow {
                    id: 2,
                    symbol: Some("updated"),
                },
                NativeRow {
                    id: 4,
                    symbol: Some("AMD"),
                },
            ],
            &options.clone().with_merge_by_names(["id"]),
        )
        .unwrap();

    assert_eq!(rows(&handle, &options), 4);
    assert_eq!(handle.read_arrow_field(&options).unwrap(), schema());
}

#[test]
fn native_row_methods_require_a_field_before_pulling() {
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct CountedRows(Arc<AtomicUsize>);

    impl Iterator for CountedRows {
        type Item = NativeRow;

        fn next(&mut self) -> Option<Self::Item> {
            self.0.fetch_add(1, Ordering::SeqCst);
            Some(NativeRow {
                id: 1,
                symbol: None,
            })
        }
    }

    for intent in ["overwrite", "append", "merge"] {
        let pulls = Arc::new(AtomicUsize::new(0));
        let mut handle = handle(&format!("native-row-no-field-{intent}.arrows"));
        let options = handle.record_options().unwrap();
        let result = match intent {
            "overwrite" => handle.overwrite_records(CountedRows(Arc::clone(&pulls)), &options),
            "append" => handle.append_records(CountedRows(Arc::clone(&pulls)), &options),
            "merge" => handle.merge_records(
                CountedRows(Arc::clone(&pulls)),
                &options.with_merge_by_names(["id"]),
            ),
            _ => unreachable!(),
        };
        let message = result.unwrap_err().to_string();
        assert!(message.contains("with_field"), "{intent}: {message}");
        assert_eq!(pulls.load(Ordering::SeqCst), 0, "{intent}");
        assert!(handle.is_empty(), "{intent}");
    }
}

#[test]
fn native_row_methods_validate_intent_before_building_or_pulling_the_iterator() {
    use std::sync::atomic::{AtomicUsize, Ordering};

    for intent in ["overwrite", "append", "merge"] {
        let pulls = Arc::new(AtomicUsize::new(0));
        let counted = Arc::clone(&pulls);
        let records = std::iter::from_fn(move || {
            counted.fetch_add(1, Ordering::SeqCst);
            Some(NativeRow {
                id: 1,
                symbol: None,
            })
        });
        let mut handle = handle(&format!("native-row-invalid-intent-{intent}.arrows"));
        let plain = handle.record_options().unwrap().with_field(schema());
        let result = match intent {
            "overwrite" => {
                handle.overwrite_records(records, &plain.clone().with_merge_by_names(["id"]))
            }
            "append" => handle.append_records(records, &plain.clone().with_merge_by_names(["id"])),
            "merge" => handle.merge_records(records, &plain),
            _ => unreachable!(),
        };

        let message = result.unwrap_err().to_string();
        assert!(message.contains("merge_by_names"), "{intent}: {message}");
        assert_eq!(pulls.load(Ordering::SeqCst), 0, "{intent}");
        assert!(handle.is_empty(), "{intent}");
    }
}

struct FallibleRow(std::result::Result<Scalar, Error>);

impl TryFrom<FallibleRow> for Scalar {
    type Error = Error;

    fn try_from(row: FallibleRow) -> std::result::Result<Self, Self::Error> {
        row.0
    }
}

struct CountedFallibleRow {
    value: std::result::Result<Scalar, Error>,
    conversions: Arc<AtomicUsize>,
}

impl TryFrom<CountedFallibleRow> for Scalar {
    type Error = Error;

    fn try_from(row: CountedFallibleRow) -> std::result::Result<Self, Self::Error> {
        row.conversions.fetch_add(1, Ordering::SeqCst);
        row.value
    }
}

#[test]
fn native_row_conversion_failure_is_typed_and_does_not_publish() {
    let mut handle = handle("native-row-failure.arrows");
    let options = handle
        .record_options()
        .unwrap()
        .with_field(schema())
        .with_batch_row_size(1);
    handle
        .overwrite_records(
            [NativeRow {
                id: 9,
                symbol: Some("kept"),
            }],
            &options,
        )
        .unwrap();
    let before = handle.as_slice().to_vec();

    let error = handle
        .append_records(
            [
                FallibleRow(Ok(Scalar::from_sequence([
                    Scalar::from(10_i64),
                    Scalar::from("not-published"),
                ]))),
                FallibleRow(Err(Error::InvalidRecord {
                    path: "$.row".into(),
                    reason: "conversion refused".into(),
                })),
            ],
            &options,
        )
        .unwrap_err();
    assert!(matches!(error, Error::InvalidRecord { .. }));
    assert_eq!(handle.as_slice(), before.as_slice());
}

#[test]
fn native_row_conversion_stops_at_each_commit_for_all_intents() {
    for intent in ["overwrite", "append", "merge"] {
        let conversions = Arc::new(AtomicUsize::new(0));
        let mut handle = PublicationProbe::new(
            &format!("native-partial-{intent}.arrows"),
            Arc::clone(&conversions),
        );
        let plain = handle.record_options().unwrap().with_field(schema());
        if intent != "overwrite" {
            handle.overwrite_arrow_reader(reader(), &plain).unwrap();
            handle.reset_publications();
        }
        let records = [
            CountedFallibleRow {
                value: Ok(Scalar::from_sequence([
                    Scalar::from(3_i64),
                    Scalar::from("committed"),
                ])),
                conversions: Arc::clone(&conversions),
            },
            CountedFallibleRow {
                value: Err(Error::InvalidRecord {
                    path: "$.row[1]".into(),
                    reason: "later native conversion failure".into(),
                }),
                conversions: Arc::clone(&conversions),
            },
        ];
        let committed = plain.clone().with_commit_row_size(1);
        let result = match intent {
            "overwrite" => handle.overwrite_records(records, &committed),
            "append" => handle.append_records(records, &committed),
            "merge" => {
                handle.merge_records(records, &committed.clone().with_merge_by_names(["id"]))
            }
            _ => unreachable!(),
        };

        let error = result.unwrap_err();
        assert!(matches!(error, Error::InvalidRecord { .. }), "{intent}");
        assert_eq!(handle.publications.load(Ordering::SeqCst), 1, "{intent}");
        assert_eq!(
            handle.pulls_when_published.lock().unwrap().as_slice(),
            [1],
            "{intent}: row two must not convert before row one publishes"
        );
        assert_eq!(conversions.load(Ordering::SeqCst), 2, "{intent}");
        assert_eq!(
            rows(&handle, &plain),
            if intent == "overwrite" { 1 } else { 3 },
            "{intent}"
        );
    }
}

#[test]
fn native_rows_align_a_non_divisible_batch_before_the_next_conversion() {
    let conversions = Arc::new(AtomicUsize::new(0));
    let mut handle = PublicationProbe::new("native-non-divisible.arrows", Arc::clone(&conversions));
    let options = handle
        .record_options()
        .unwrap()
        .with_field(schema())
        .with_batch_row_size(2)
        .with_commit_row_size(3);
    let mut records = Vec::new();
    for id in 1..=3_i64 {
        records.push(CountedFallibleRow {
            value: Ok(Scalar::from_sequence([
                Scalar::from(id),
                Scalar::from("committed"),
            ])),
            conversions: Arc::clone(&conversions),
        });
    }
    records.push(CountedFallibleRow {
        value: Err(Error::InvalidRecord {
            path: "$.row[3]".into(),
            reason: "conversion after a non-divisible cadence".into(),
        }),
        conversions: Arc::clone(&conversions),
    });

    let error = handle.overwrite_records(records, &options).unwrap_err();
    assert!(matches!(error, Error::InvalidRecord { .. }));
    assert_eq!(handle.publications.load(Ordering::SeqCst), 1);
    assert_eq!(
        handle.pulls_when_published.lock().unwrap().as_slice(),
        [3],
        "row four must not convert before the three-row cadence publishes"
    );
    assert_eq!(conversions.load(Ordering::SeqCst), 4);
    assert_eq!(rows(&handle, &options), 3);
}

#[test]
fn native_rows_stop_at_the_global_row_limit_without_one_extra_pull() {
    let pulls = Arc::new(AtomicUsize::new(0));
    let counted = Arc::clone(&pulls);
    let records = std::iter::from_fn(move || {
        let id = counted.fetch_add(1, Ordering::SeqCst) as i64;
        Some(NativeRow {
            id,
            symbol: Some("bounded"),
        })
    });
    let mut handle = handle("native-global-row-limit.arrows");
    let options = handle
        .record_options()
        .unwrap()
        .with_field(schema())
        .with_batch_row_size(2)
        .with_max_row_size(3);

    handle.overwrite_records(records, &options).unwrap();

    assert_eq!(pulls.load(Ordering::SeqCst), 3);
    assert_eq!(rows(&handle, &options), 3);
}

#[test]
fn empty_native_row_intents_keep_overwrite_schema_and_make_append_merge_no_ops() {
    let mut missing = handle("empty-native-row-append.arrows");
    let options = missing.record_options().unwrap().with_field(schema());
    missing
        .append_records(std::iter::empty::<NativeRow>(), &options)
        .unwrap();
    assert!(missing.is_empty());

    let mut handle = handle("empty-native-row-overwrite.arrows");
    handle
        .overwrite_records(std::iter::empty::<NativeRow>(), &options)
        .unwrap();
    assert_eq!(rows(&handle, &options), 0);
    assert_eq!(handle.read_arrow_field(&options).unwrap(), schema());
    let before = handle.as_slice().to_vec();

    handle
        .append_records(
            std::iter::empty::<NativeRow>(),
            &options.clone().with_select_by_names(["absent"]),
        )
        .unwrap();
    handle
        .merge_records(
            std::iter::empty::<NativeRow>(),
            &options
                .clone()
                .with_merge_by_names(["id"])
                .with_select_by_names(["absent"]),
        )
        .unwrap();
    assert_eq!(handle.as_slice(), before.as_slice());
}

#[test]
fn an_empty_record_batch_overwrite_keeps_its_field_and_no_rows() {
    let mut handle = handle("empty-record-batch.arrows");
    let options = handle.record_options().unwrap();
    let empty = RecordBatch::new_empty(crate::arrow::arrow_schema_from_field(&schema()).unwrap());

    handle.overwrite_arrow_batch(empty, &options).unwrap();

    assert_eq!(rows(&handle, &options), 0);
    assert_eq!(handle.read_arrow_field(&options).unwrap(), schema());
}

#[test]
fn empty_append_and_merge_are_byte_for_byte_no_ops() {
    let mut missing = handle("empty-no-op.arrows");
    let options = missing.record_options().unwrap().with_field(schema());
    missing
        .append_arrow_reader(
            crate::arrow::batch_reader(
                crate::arrow::arrow_schema_from_field(&schema()).unwrap(),
                [],
            ),
            &options.clone().with_select_by_names(["absent"]),
        )
        .unwrap();
    assert!(missing.is_empty(), "an empty append must not create bytes");

    missing.overwrite_arrow_reader(reader(), &options).unwrap();
    let before = missing.as_slice().to_vec();
    let zero = RecordBatch::new_empty(crate::arrow::arrow_schema_from_field(&schema()).unwrap());
    missing
        .merge_arrow_reader(
            crate::arrow::batch_reader(zero.schema(), [zero]),
            &options
                .clone()
                .with_merge_by_names(["id"])
                .with_select_by_names(["absent"]),
        )
        .unwrap();
    assert_eq!(missing.as_slice(), before.as_slice());
}

#[test]
fn appending_casts_incoming_batches_to_the_target_shape() {
    let mut handle = handle("cast-append.arrows");
    let options = handle.record_options().unwrap().with_field(schema());
    handle.overwrite_arrow_reader(reader(), &options).unwrap();

    // The incoming batch merely fits: `id` is narrower and the columns are
    // the other way round.
    let loose = DataType::from_fields([
        DataType::Utf8.nullable_field("symbol"),
        DataType::Int32.required_field("id"),
    ])
    .unwrap()
    .required_field("row");
    let incoming = RecordBatch::try_new(
        crate::arrow::arrow_schema_from_field(&loose).unwrap(),
        vec![
            Arc::new(StringArray::from(vec![Some("MSFT")])),
            Arc::new(arrow_array::Int32Array::from(vec![3])),
        ],
    )
    .unwrap();

    handle
        .append_arrow_reader(
            crate::arrow::batch_reader(incoming.schema(), [incoming]),
            &options,
        )
        .unwrap();

    let batches = handle
        .read_arrow_reader(&options)
        .unwrap()
        .map(std::result::Result::unwrap)
        .collect::<Vec<_>>();
    assert_eq!(batches.len(), 2);
    assert_eq!(batches[1].schema(), batches[0].schema());
    assert_eq!(batches[1].num_rows(), 1);
}

#[test]
fn a_cast_that_cannot_be_planned_leaves_the_resource_alone() {
    let mut handle = handle("failed-append.arrows");
    let options = handle.record_options().unwrap().with_field(schema());
    handle.overwrite_arrow_reader(reader(), &options).unwrap();
    let before = handle.as_slice().to_vec();

    // Text that is not a number cannot become the declared Int64, and this
    // write is strict, so the append fails while the batches are being
    // encoded - before anything is published.
    let hostile = DataType::from_fields([DataType::Utf8.required_field("id")])
        .unwrap()
        .required_field("row");
    let incoming = RecordBatch::try_new(
        crate::arrow::arrow_schema_from_field(&hostile).unwrap(),
        vec![Arc::new(StringArray::from(vec!["not a number"]))],
    )
    .unwrap();

    let message = handle
        .append_arrow_reader(
            crate::arrow::batch_reader(incoming.schema(), [incoming]),
            &options,
        )
        .unwrap_err()
        .to_string();

    // The core failure is reported as itself rather than as the Arrow
    // envelope it had to travel through the reader inside.
    assert!(!message.contains("External error"), "{message}");
    assert_eq!(handle.as_slice(), before.as_slice());
}

#[test]
fn a_row_limit_bounds_a_read_at_below_and_above_the_stored_count() {
    let mut handle = handle("row-limited.arrows");
    let options = handle.record_options().unwrap().with_field(schema());
    handle.overwrite_arrow_reader(reader(), &options).unwrap();

    // The bound is exact: one below slices, the count itself keeps
    // everything, and one above changes nothing.
    assert_eq!(rows(&handle, &options.clone().with_max_row_size(1)), 1);
    assert_eq!(rows(&handle, &options.clone().with_max_row_size(2)), 2);
    assert_eq!(rows(&handle, &options.clone().with_max_row_size(3)), 2);
}

#[test]
fn overwrite_refuses_a_match_key_and_a_limited_merge_names_both_settings() {
    let mut handle = handle("limited-merge.arrows");
    let keyed = handle
        .record_options()
        .unwrap()
        .with_field(schema())
        .with_merge_by_names(["id"]);
    let before = handle.as_slice().to_vec();

    // The operation carries intent: overwrite never silently becomes a
    // merge because its options happen to carry keys.
    let message = handle
        .overwrite_arrow_reader(reader(), &keyed)
        .unwrap_err()
        .to_string();
    assert!(message.contains("write mode overwrite"), "{message}");
    assert!(message.contains("merge_by_names"), "{message}");
    assert_eq!(handle.as_slice(), before.as_slice());

    // A truncated merge would update the matched keys it kept and
    // silently drop the rest, so the combination is refused by name
    // before a single row moves.
    let limited = keyed.with_max_row_size(1);
    let message = handle
        .merge_arrow_reader(reader(), &limited)
        .unwrap_err()
        .to_string();
    assert!(message.contains("max_row_size = 1"), "{message}");
    assert!(message.contains("merge_by_names [\"id\"]"), "{message}");
    assert_eq!(handle.as_slice(), before.as_slice());
}

#[test]
fn merge_refuses_an_empty_match_key_before_pulling_the_reader() {
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct Counting {
        schema: arrow_schema::SchemaRef,
        pulls: Arc<AtomicUsize>,
    }

    impl Iterator for Counting {
        type Item = std::result::Result<RecordBatch, arrow_schema::ArrowError>;

        fn next(&mut self) -> Option<Self::Item> {
            self.pulls.fetch_add(1, Ordering::SeqCst);
            Some(Ok(batch()))
        }
    }

    impl RecordBatchReader for Counting {
        fn schema(&self) -> arrow_schema::SchemaRef {
            Arc::clone(&self.schema)
        }
    }

    let pulls = Arc::new(AtomicUsize::new(0));
    let reader: BatchReader = Box::new(Counting {
        schema: crate::arrow::arrow_schema_from_field(&schema()).unwrap(),
        pulls: Arc::clone(&pulls),
    });
    let mut handle = handle("missing-merge-key.arrows");
    let options = handle.record_options().unwrap().with_field(schema());
    let message = handle
        .merge_arrow_reader(reader, &options)
        .unwrap_err()
        .to_string();

    assert!(message.contains("requires at least one"), "{message}");
    assert!(message.contains("merge_by_names"), "{message}");
    assert_eq!(pulls.load(Ordering::SeqCst), 0);
    assert!(handle.is_empty());
}
