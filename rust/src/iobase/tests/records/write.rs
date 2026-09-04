use super::*;

#[test]
fn the_handles_media_type_picks_the_record_encoding() {
    assert!(matches!(
        handle("t.arrows").record_options().unwrap(),
        RecordOptions::Ipc(_)
    ));
    #[cfg(feature = "parquet")]
    assert!(matches!(
        handle("t.parquet").record_options().unwrap(),
        RecordOptions::Parquet(_)
    ));

    // An encoding with no implementation is named rather than guessed.
    let message = handle("t.csv").record_options().unwrap_err().to_string();
    assert!(message.contains("text/csv"), "{message}");
}

#[test]
fn batches_round_trip_through_a_bare_handle() {
    let mut names = vec!["t.arrows", "t.arrows.zst"];
    if cfg!(feature = "parquet") {
        names.push("t.parquet");
    }

    for name in names {
        let mut handle = handle(name);
        let options = handle
            .record_options()
            .unwrap()
            .with_field(schema())
            .with_safe(true);

        handle
            .overwrite_arrow_reader(reader(), &options)
            .unwrap_or_else(|error| panic!("{name}: {error}"));
        assert_eq!(rows(&handle, &options), 2, "{name}");
        assert_eq!(
            handle.read_arrow_field(&options).unwrap(),
            schema(),
            "{name}"
        );
    }
}

#[test]
fn a_write_stores_the_schema_its_reader_declares() {
    let mut handle = handle("declared.arrows");
    let options = handle.record_options().unwrap();

    // The write path takes a reader and nothing else, so with nothing
    // declared and nothing stored the reader's own schema is what the
    // resource ends up holding.
    handle.overwrite_arrow_reader(reader(), &options).unwrap();

    assert_eq!(handle.read_arrow_field(&options).unwrap(), schema());
    assert_eq!(
        handle
            .read_arrow_reader(&options)
            .unwrap()
            .schema()
            .fields()
            .len(),
        2
    );
}

#[test]
fn an_overwrite_keeps_the_schema_the_resource_already_stores() {
    let mut handle = handle("stable.arrows");
    let options = handle.record_options().unwrap();
    handle.overwrite_arrow_reader(reader(), &options).unwrap();

    // The incoming rows declare `id` as text and drop `symbol` entirely. An
    // overwrite replaces rows, so the stored columns survive it and the
    // text is cast back into the stored Int64.
    let loose = DataType::from_fields([DataType::Utf8.required_field("id")])
        .unwrap()
        .required_field("row");
    let incoming = RecordBatch::try_new(
        crate::arrow::arrow_schema_from_field(&loose).unwrap(),
        vec![Arc::new(StringArray::from(vec!["7"]))],
    )
    .unwrap();
    handle
        .overwrite_arrow_reader(
            crate::arrow::batch_reader(incoming.schema(), [incoming]),
            &options,
        )
        .unwrap();

    assert_eq!(handle.read_arrow_field(&options).unwrap(), schema());
    assert_eq!(rows(&handle, &options), 1);
}

#[test]
fn a_missing_resource_reads_as_empty_rather_than_failing() {
    let handle = Buffer::new().with_media_type(MimeType::ARROW_STREAM.into());
    let options = handle.record_options().unwrap().with_field(schema());

    assert_eq!(handle.read_arrow_reader(&options).unwrap().count(), 0);
    assert_eq!(handle.read_arrow_field(&options).unwrap(), schema());
}

#[test]
fn appending_reads_adds_and_rewrites() {
    let mut handle = handle("append.arrows");
    let options = handle.record_options().unwrap().with_field(schema());

    // Appending to nothing simply writes.
    handle.append_arrow_reader(reader(), &options).unwrap();
    assert_eq!(rows(&handle, &options), 2);

    handle.append_arrow_reader(reader(), &options).unwrap();
    assert_eq!(rows(&handle, &options), 4);
}

#[test]
fn commit_row_size_controls_exact_publication_counts() {
    for (label, cadence, expected) in [
        ("unset", None, 1),
        ("one", Some(1), 4),
        ("across-batches", Some(3), 2),
        ("larger-than-stream", Some(10), 1),
    ] {
        let pulls = Arc::new(AtomicUsize::new(0));
        let mut handle = PublicationProbe::new(
            &format!("commit-publications-{label}.arrows"),
            Arc::clone(&pulls),
        );
        let mut options = handle.record_options().unwrap().with_field(schema());
        options.set_commit_row_size(cadence);
        let source = crate::arrow::batch_reader(
            crate::arrow::arrow_schema_from_field(&schema()).unwrap(),
            [rows_batch(&[1, 2]), rows_batch(&[3, 4])],
        );

        handle.overwrite_arrow_reader(source, &options).unwrap();

        assert_eq!(
            handle.publications.load(Ordering::SeqCst),
            expected,
            "{label}"
        );
        assert_eq!(rows(&handle, &options), 4, "{label}");
    }
}

#[test]
fn every_write_intent_retains_its_intent_for_each_commit() {
    for intent in ["overwrite", "append", "merge"] {
        let pulls = Arc::new(AtomicUsize::new(0));
        let mut handle = PublicationProbe::new(
            &format!("commit-intent-{intent}.arrows"),
            Arc::clone(&pulls),
        );
        let plain = handle.record_options().unwrap().with_field(schema());
        handle
            .overwrite_arrow_reader(
                crate::arrow::batch_reader(
                    crate::arrow::arrow_schema_from_field(&schema()).unwrap(),
                    [rows_batch(&[1, 2])],
                ),
                &plain,
            )
            .unwrap();
        handle.reset_publications();

        let options = plain.clone().with_commit_row_size(2);
        let incoming = crate::arrow::batch_reader(
            crate::arrow::arrow_schema_from_field(&schema()).unwrap(),
            [rows_batch(&[1, 3, 4, 5])],
        );
        match intent {
            "overwrite" => handle.overwrite_arrow_reader(incoming, &options).unwrap(),
            "append" => handle.append_arrow_reader(incoming, &options).unwrap(),
            "merge" => handle
                .merge_arrow_reader(incoming, &options.with_merge_by_names(["id"]))
                .unwrap(),
            _ => unreachable!(),
        }

        assert_eq!(handle.publications.load(Ordering::SeqCst), 2, "{intent}");
        let expected_rows = match intent {
            "overwrite" => 4,
            "append" => 6,
            "merge" => 5,
            _ => unreachable!(),
        };
        assert_eq!(rows(&handle, &plain), expected_rows, "{intent}");
    }
}

#[test]
fn held_batch_and_native_row_adapters_inherit_commit_boundaries() {
    let pulls = Arc::new(AtomicUsize::new(0));
    let mut batch_handle = PublicationProbe::new("commit-batch.arrows", Arc::clone(&pulls));
    let options = batch_handle
        .record_options()
        .unwrap()
        .with_field(schema())
        .with_commit_row_size(1);
    batch_handle
        .overwrite_arrow_batch(rows_batch(&[1, 2]), &options)
        .unwrap();
    assert_eq!(batch_handle.publications.load(Ordering::SeqCst), 2);

    let mut row_handle = PublicationProbe::new("commit-rows.arrows", pulls);
    row_handle
        .overwrite_records(
            [
                NativeRow {
                    id: 1,
                    symbol: Some("AAPL"),
                },
                NativeRow {
                    id: 2,
                    symbol: Some("MSFT"),
                },
            ],
            &options,
        )
        .unwrap();
    assert_eq!(row_handle.publications.load(Ordering::SeqCst), 2);
    assert_eq!(rows(&row_handle, &options), 2);
}

#[test]
fn zero_commit_row_size_is_rejected_before_any_input_pull() {
    for intent in ["overwrite", "append", "merge"] {
        let pulls = Arc::new(AtomicUsize::new(0));
        let mut handle =
            PublicationProbe::new(&format!("zero-commit-{intent}.arrows"), Arc::clone(&pulls));
        let options = handle
            .record_options()
            .unwrap()
            .with_field(schema())
            .with_commit_row_size(0);
        let source = counted_source(Arc::clone(&pulls), [Ok(rows_batch(&[1]))]);
        let result = match intent {
            "overwrite" => handle.overwrite_arrow_reader(source, &options),
            "append" => handle.append_arrow_reader(source, &options),
            "merge" => {
                handle.merge_arrow_reader(source, &options.clone().with_merge_by_names(["id"]))
            }
            _ => unreachable!(),
        };

        let message = result.unwrap_err().to_string();
        assert!(message.contains("commit_row_size"), "{intent}: {message}");
        assert_eq!(pulls.load(Ordering::SeqCst), 0, "{intent}");
        assert_eq!(handle.publications.load(Ordering::SeqCst), 0, "{intent}");
    }

    let pulls = Arc::new(AtomicUsize::new(0));
    let counted = Arc::clone(&pulls);
    let records = std::iter::from_fn(move || {
        counted.fetch_add(1, Ordering::SeqCst);
        Some(NativeRow {
            id: 1,
            symbol: None,
        })
    });
    let mut handle = handle("zero-commit-native.arrows");
    let options = handle
        .record_options()
        .unwrap()
        .with_field(schema())
        .with_commit_row_size(0);
    let message = handle
        .overwrite_records(records, &options)
        .unwrap_err()
        .to_string();
    assert!(message.contains("commit_row_size"), "{message}");
    assert_eq!(pulls.load(Ordering::SeqCst), 0);
}

#[test]
fn empty_append_and_merge_do_not_touch_the_destination() {
    let source_pulls = Arc::new(AtomicUsize::new(0));
    let mut handle = PublicationProbe::new("empty-no-touch.arrows", source_pulls);
    let options = handle.record_options().unwrap().with_field(schema());
    let touches = Arc::clone(&handle.destination_touches);
    // Option discovery is outside the write; count only destination work
    // performed after the empty source crosses the primitive boundary.
    touches.store(0, Ordering::SeqCst);
    let empty = || {
        crate::arrow::batch_reader(
            crate::arrow::arrow_schema_from_field(&schema()).unwrap(),
            [],
        )
    };

    handle.append_arrow_reader(empty(), &options).unwrap();
    handle
        .merge_arrow_reader(empty(), &options.with_merge_by_names(["id"]))
        .unwrap();

    assert_eq!(touches.load(Ordering::SeqCst), 0);
    assert_eq!(handle.publications.load(Ordering::SeqCst), 0);
}

#[test]
fn zero_append_limits_and_invalid_merge_limits_do_not_pull() {
    for options in [
        handle("limit-options.arrows")
            .record_options()
            .unwrap()
            .with_field(schema())
            .with_max_row_size(0),
        handle("limit-options.arrows")
            .record_options()
            .unwrap()
            .with_field(schema())
            .with_max_byte_size(0),
    ] {
        let pulls = Arc::new(AtomicUsize::new(0));
        let source = counted_source(Arc::clone(&pulls), [Ok(rows_batch(&[1]))]);
        let mut destination = handle("zero-limit-append.arrows");
        destination.append_arrow_reader(source, &options).unwrap();
        assert_eq!(pulls.load(Ordering::SeqCst), 0);
    }

    for limited in [
        handle("merge-limit-options.arrows")
            .record_options()
            .unwrap()
            .with_field(schema())
            .with_merge_by_names(["id"])
            .with_max_row_size(1),
        handle("merge-limit-options.arrows")
            .record_options()
            .unwrap()
            .with_field(schema())
            .with_merge_by_names(["id"])
            .with_max_byte_size(1),
    ] {
        let pulls = Arc::new(AtomicUsize::new(0));
        let source = counted_source(Arc::clone(&pulls), [Ok(rows_batch(&[1]))]);
        let mut destination = handle("invalid-limit-merge.arrows");
        let message = destination
            .merge_arrow_reader(source, &limited)
            .unwrap_err()
            .to_string();
        assert!(message.contains("merge_by_names"), "{message}");
        assert_eq!(pulls.load(Ordering::SeqCst), 0);
    }
}

#[test]
fn a_later_source_failure_leaves_each_successful_prefix_visible() {
    for intent in ["overwrite", "append", "merge"] {
        let pulls = Arc::new(AtomicUsize::new(0));
        let mut handle = PublicationProbe::new(
            &format!("partial-commit-{intent}.arrows"),
            Arc::clone(&pulls),
        );
        let plain = handle.record_options().unwrap().with_field(schema());
        if intent != "overwrite" {
            handle
                .overwrite_arrow_reader(
                    crate::arrow::batch_reader(
                        crate::arrow::arrow_schema_from_field(&schema()).unwrap(),
                        [rows_batch(&[1, 2])],
                    ),
                    &plain,
                )
                .unwrap();
            handle.reset_publications();
        }
        let options = plain.clone().with_commit_row_size(2);
        let source = counted_source(
            Arc::clone(&pulls),
            [
                Ok(rows_batch(&[2, 3])),
                Ok(rows_batch(&[99])),
                Err(ArrowError::ComputeError("later source failure".into())),
            ],
        );
        let result = match intent {
            "overwrite" => handle.overwrite_arrow_reader(source, &options),
            "append" => handle.append_arrow_reader(source, &options),
            "merge" => {
                handle.merge_arrow_reader(source, &options.clone().with_merge_by_names(["id"]))
            }
            _ => unreachable!(),
        };

        let message = result.unwrap_err().to_string();
        assert!(
            message.contains("later source failure"),
            "{intent}: {message}"
        );
        assert_eq!(handle.publications.load(Ordering::SeqCst), 1, "{intent}");
        assert_eq!(
            handle.pulls_when_published.lock().unwrap().as_slice(),
            [1],
            "{intent}: the second batch must not be pulled before commit one publishes"
        );
        assert_eq!(
            pulls.load(Ordering::SeqCst),
            3,
            "{intent}: the one-row second cadence is discarded when its next pull fails"
        );
        let expected_rows = match intent {
            "overwrite" => 2,
            "append" => 4,
            "merge" => 3,
            _ => unreachable!(),
        };
        assert_eq!(rows(&handle, &plain), expected_rows, "{intent}");
    }
}

#[test]
fn a_second_publication_failure_keeps_the_first_commit_visible() {
    let pulls = Arc::new(AtomicUsize::new(0));
    let mut handle = PublicationProbe::new("second-publication-failure.arrows", pulls);
    handle.fail_on_publication(2);
    let options = handle
        .record_options()
        .unwrap()
        .with_field(schema())
        .with_commit_row_size(2);
    let source = crate::arrow::batch_reader(
        crate::arrow::arrow_schema_from_field(&schema()).unwrap(),
        [rows_batch(&[1, 2, 3, 4])],
    );

    let message = handle
        .overwrite_arrow_reader(source, &options)
        .unwrap_err()
        .to_string();

    assert!(message.contains("publication 2 refused"), "{message}");
    assert_eq!(handle.publications.load(Ordering::SeqCst), 2);
    assert_eq!(rows(&handle, &options), 2);
}

#[test]
fn resumed_write_publishes_complete_cadences_and_abort_drops_only_the_remainder() {
    let pulls = Arc::new(AtomicUsize::new(0));
    let mut handle = PublicationProbe::new("resumed-write.arrows", pulls);
    let options = handle
        .record_options()
        .unwrap()
        .with_field(schema())
        .with_commit_row_size(3);
    let mut session = ArrowWriteSession::overwrite(&options).unwrap();

    assert!(
        session
            .push(
                &mut handle,
                crate::arrow::batch_reader(
                    crate::arrow::arrow_schema_from_field(&schema()).unwrap(),
                    [rows_batch(&[1, 2])],
                ),
            )
            .unwrap()
    );
    assert_eq!(handle.publications.load(Ordering::SeqCst), 0);

    assert!(
        session
            .push(
                &mut handle,
                crate::arrow::batch_reader(
                    crate::arrow::arrow_schema_from_field(&schema()).unwrap(),
                    [rows_batch(&[3])],
                ),
            )
            .unwrap()
    );
    assert_eq!(handle.publications.load(Ordering::SeqCst), 1);
    assert_eq!(rows(&handle, &options), 3);

    session
        .push(
            &mut handle,
            crate::arrow::batch_reader(
                crate::arrow::arrow_schema_from_field(&schema()).unwrap(),
                [rows_batch(&[4])],
            ),
        )
        .unwrap();
    session.abort();
    assert_eq!(handle.publications.load(Ordering::SeqCst), 1);
    assert_eq!(rows(&handle, &options), 3);
}

#[test]
fn resumed_write_keeps_global_limits_and_stops_before_another_chunk_pull() {
    let pulls = Arc::new(AtomicUsize::new(0));
    let mut handle = PublicationProbe::new("resumed-limit.arrows", Arc::clone(&pulls));
    let options = handle
        .record_options()
        .unwrap()
        .with_field(schema())
        .with_commit_row_size(2)
        .with_max_row_size(3);
    let mut session = ArrowWriteSession::overwrite(&options).unwrap();

    assert!(
        session
            .push(
                &mut handle,
                counted_source(Arc::clone(&pulls), [Ok(rows_batch(&[1, 2]))]),
            )
            .unwrap()
    );
    let second = counted_source(
        Arc::clone(&pulls),
        [Ok(rows_batch(&[3, 4])), Ok(rows_batch(&[99]))],
    );
    assert!(!session.push(&mut handle, second).unwrap());
    session.finish(&mut handle).unwrap();

    assert_eq!(pulls.load(Ordering::SeqCst), 2);
    assert_eq!(handle.publications.load(Ordering::SeqCst), 2);
    assert_eq!(rows(&handle, &options), 3);
}

#[test]
fn resumed_zero_limits_need_no_source_and_only_overwrite_publishes() {
    let pulls = Arc::new(AtomicUsize::new(0));
    let mut handle = PublicationProbe::new("resumed-zero.arrows", pulls);
    let base = handle
        .record_options()
        .unwrap()
        .with_field(schema())
        .with_commit_row_size(2)
        .with_max_row_size(0);
    handle.destination_touches.store(0, Ordering::SeqCst);

    let mut append = ArrowWriteSession::append(&base).unwrap();
    append.finish(&mut handle).unwrap();
    assert_eq!(handle.publications.load(Ordering::SeqCst), 0);
    assert_eq!(handle.destination_touches.load(Ordering::SeqCst), 0);

    let mut overwrite = ArrowWriteSession::overwrite(&base).unwrap();
    overwrite.finish(&mut handle).unwrap();
    assert_eq!(handle.publications.load(Ordering::SeqCst), 1);
    assert_eq!(rows(&handle, &base), 0);
    assert_eq!(handle.read_arrow_field(&base).unwrap(), schema());
}

#[test]
fn resumed_sessions_keep_append_and_merge_intent_for_every_cadence() {
    let pulls = Arc::new(AtomicUsize::new(0));
    let mut handle = PublicationProbe::new("resumed-intents.arrows", pulls);
    let plain = handle.record_options().unwrap().with_field(schema());
    handle
        .overwrite_arrow_reader(
            crate::arrow::batch_reader(
                crate::arrow::arrow_schema_from_field(&schema()).unwrap(),
                [rows_batch(&[1, 2])],
            ),
            &plain,
        )
        .unwrap();

    handle.reset_publications();
    let append_options = plain.clone().with_commit_row_size(1);
    let mut append = ArrowWriteSession::append(&append_options).unwrap();
    append
        .push(
            &mut handle,
            crate::arrow::batch_reader(
                crate::arrow::arrow_schema_from_field(&schema()).unwrap(),
                [rows_batch(&[3, 4])],
            ),
        )
        .unwrap();
    append.finish(&mut handle).unwrap();
    assert_eq!(handle.publications.load(Ordering::SeqCst), 2);
    assert_eq!(rows(&handle, &plain), 4);

    handle.reset_publications();
    let merge_options = plain
        .clone()
        .with_commit_row_size(1)
        .with_merge_by_names(["id"]);
    let mut merge = ArrowWriteSession::merge(&merge_options).unwrap();
    merge
        .push(
            &mut handle,
            crate::arrow::batch_reader(
                crate::arrow::arrow_schema_from_field(&schema()).unwrap(),
                [rows_batch(&[2, 5])],
            ),
        )
        .unwrap();
    merge.finish(&mut handle).unwrap();
    assert_eq!(handle.publications.load(Ordering::SeqCst), 2);
    assert_eq!(rows(&handle, &plain), 5);
}

#[test]
fn resumed_session_covers_large_cadence_multiple_commits_and_terminal_reuse() {
    let pulls = Arc::new(AtomicUsize::new(0));
    let mut large = PublicationProbe::new("resumed-large-cadence.arrows", Arc::clone(&pulls));
    let large_options = large
        .record_options()
        .unwrap()
        .with_field(schema())
        .with_commit_row_size(10);
    let mut session = ArrowWriteSession::overwrite(&large_options).unwrap();
    session
        .push(
            &mut large,
            crate::arrow::batch_reader(
                crate::arrow::arrow_schema_from_field(&schema()).unwrap(),
                [rows_batch(&[1, 2])],
            ),
        )
        .unwrap();
    assert_eq!(large.publications.load(Ordering::SeqCst), 0);
    session.finish(&mut large).unwrap();
    assert_eq!(large.publications.load(Ordering::SeqCst), 1);
    assert_eq!(rows(&large, &large_options), 2);
    let message = session
        .push(
            &mut large,
            crate::arrow::batch_reader(
                crate::arrow::arrow_schema_from_field(&schema()).unwrap(),
                [rows_batch(&[3])],
            ),
        )
        .unwrap_err()
        .to_string();
    assert!(message.contains("cannot be reused"), "{message}");

    let mut exact = PublicationProbe::new("resumed-multiple.arrows", pulls);
    let exact_options = exact
        .record_options()
        .unwrap()
        .with_field(schema())
        .with_commit_row_size(2);
    let mut exact_session = ArrowWriteSession::overwrite(&exact_options).unwrap();
    exact_session
        .push(
            &mut exact,
            crate::arrow::batch_reader(
                crate::arrow::arrow_schema_from_field(&schema()).unwrap(),
                [rows_batch(&[1, 2, 3, 4])],
            ),
        )
        .unwrap();
    exact_session.finish(&mut exact).unwrap();
    assert_eq!(exact.publications.load(Ordering::SeqCst), 2);
    assert_eq!(rows(&exact, &exact_options), 4);
}

#[test]
fn resumed_session_fuses_on_schema_source_and_publication_failures() {
    let pulls = Arc::new(AtomicUsize::new(0));
    let mut mismatch = PublicationProbe::new("resumed-schema.arrows", Arc::clone(&pulls));
    let options = mismatch
        .record_options()
        .unwrap()
        .with_field(schema())
        .with_commit_row_size(2);
    let mut session = ArrowWriteSession::overwrite(&options).unwrap();
    session
        .push(
            &mut mismatch,
            crate::arrow::batch_reader(
                crate::arrow::arrow_schema_from_field(&schema()).unwrap(),
                [rows_batch(&[1])],
            ),
        )
        .unwrap();
    let other = DataType::from_fields([DataType::Utf8.required_field("id")])
        .unwrap()
        .required_field("row");
    let other_batch = RecordBatch::try_new(
        crate::arrow::arrow_schema_from_field(&other).unwrap(),
        vec![Arc::new(StringArray::from(vec!["2"]))],
    )
    .unwrap();
    let message = session
        .push(
            &mut mismatch,
            crate::arrow::batch_reader(other_batch.schema(), [other_batch]),
        )
        .unwrap_err()
        .to_string();
    assert!(message.contains("later chunk schema"), "{message}");
    assert_eq!(mismatch.publications.load(Ordering::SeqCst), 0);

    let mut source_failure =
        PublicationProbe::new("resumed-source-error.arrows", Arc::clone(&pulls));
    let source_options = source_failure
        .record_options()
        .unwrap()
        .with_field(schema())
        .with_commit_row_size(2);
    let mut source_session = ArrowWriteSession::overwrite(&source_options).unwrap();
    let error = source_session
        .push(
            &mut source_failure,
            counted_source(
                Arc::clone(&pulls),
                [
                    Ok(rows_batch(&[1, 2])),
                    Err(ArrowError::ComputeError("resumed source failed".into())),
                ],
            ),
        )
        .unwrap_err();
    assert!(error.to_string().contains("resumed source failed"));
    assert_eq!(source_failure.publications.load(Ordering::SeqCst), 1);
    assert_eq!(rows(&source_failure, &source_options), 2);

    let mut publication = PublicationProbe::new("resumed-publication-error.arrows", pulls);
    publication.fail_on_publication(2);
    let publication_options = publication
        .record_options()
        .unwrap()
        .with_field(schema())
        .with_commit_row_size(1);
    let mut publication_session = ArrowWriteSession::overwrite(&publication_options).unwrap();
    let error = publication_session
        .push(
            &mut publication,
            crate::arrow::batch_reader(
                crate::arrow::arrow_schema_from_field(&schema()).unwrap(),
                [rows_batch(&[1, 2])],
            ),
        )
        .unwrap_err();
    assert!(error.to_string().contains("publication 2 refused"));
    assert_eq!(publication.publications.load(Ordering::SeqCst), 2);
    assert_eq!(rows(&publication, &publication_options), 1);
}

#[test]
fn resumed_leaf_keeps_the_target_captured_before_an_external_replacement() {
    let pulls = Arc::new(AtomicUsize::new(0));
    let mut handle = PublicationProbe::new("resumed-stable-target.arrows", pulls);
    let options = handle
        .record_options()
        .unwrap()
        .with_field(schema())
        .with_commit_row_size(1);
    let mut session = ArrowWriteSession::overwrite(&options).unwrap();
    session
        .push(
            &mut handle,
            crate::arrow::batch_reader(
                crate::arrow::arrow_schema_from_field(&schema()).unwrap(),
                [rows_batch(&[1])],
            ),
        )
        .unwrap();

    handle.clear().unwrap();
    let loose = DataType::from_fields([DataType::Utf8.required_field("id")])
        .unwrap()
        .required_field("other");
    let loose_batch = RecordBatch::try_new(
        crate::arrow::arrow_schema_from_field(&loose).unwrap(),
        vec![Arc::new(StringArray::from(vec!["9"]))],
    )
    .unwrap();
    let external = handle.record_options().unwrap();
    handle
        .overwrite_arrow_reader(
            crate::arrow::batch_reader(loose_batch.schema(), [loose_batch]),
            &external,
        )
        .unwrap();
    let replaced = handle.read_arrow_field(&external).unwrap();
    assert_eq!(replaced.dtype(), loose.dtype());
    assert_ne!(replaced, schema());

    session
        .push(
            &mut handle,
            crate::arrow::batch_reader(
                crate::arrow::arrow_schema_from_field(&schema()).unwrap(),
                [rows_batch(&[2])],
            ),
        )
        .unwrap();
    session.finish(&mut handle).unwrap();

    assert_eq!(handle.read_arrow_field(&options).unwrap(), schema());
    assert_eq!(rows(&handle, &options), 2);
}

#[test]
fn bounded_empty_intents_publish_only_overwrite() {
    let pulls = Arc::new(AtomicUsize::new(0));
    let mut handle = PublicationProbe::new("bounded-empty.arrows", pulls);
    let plain = handle.record_options().unwrap().with_field(schema());
    handle.overwrite_arrow_reader(reader(), &plain).unwrap();
    handle.reset_publications();
    let bounded = plain.clone().with_commit_row_size(2);
    let empty = || {
        crate::arrow::batch_reader(
            crate::arrow::arrow_schema_from_field(&schema()).unwrap(),
            [],
        )
    };

    handle.append_arrow_reader(empty(), &bounded).unwrap();
    handle
        .merge_arrow_reader(empty(), &bounded.clone().with_merge_by_names(["id"]))
        .unwrap();
    assert_eq!(handle.publications.load(Ordering::SeqCst), 0);
    assert_eq!(rows(&handle, &plain), 2);

    handle.overwrite_arrow_reader(empty(), &bounded).unwrap();
    assert_eq!(handle.publications.load(Ordering::SeqCst), 1);
    assert_eq!(rows(&handle, &plain), 0);
    assert_eq!(handle.read_arrow_field(&plain).unwrap(), schema());
}

#[cfg(feature = "parquet")]
#[test]
fn the_three_methods_behave_the_same_way_on_parquet() {
    let mut handle = handle("three.parquet");
    let options = handle.record_options().unwrap().with_field(schema());

    handle.append_arrow_reader(reader(), &options).unwrap();
    handle.overwrite_arrow_reader(reader(), &options).unwrap();
    handle.append_arrow_reader(reader(), &options).unwrap();

    assert_eq!(rows(&handle, &options), 4);
}

#[test]
fn record_batch_adapters_route_to_each_explicit_reader_primitive() {
    let mut handle = handle("record-batch-adapters.arrows");
    let options = handle.record_options().unwrap().with_field(schema());

    handle.overwrite_arrow_batch(batch(), &options).unwrap();
    handle.append_arrow_batch(batch(), &options).unwrap();
    assert_eq!(rows(&handle, &options), 4);

    let merging = options.clone().with_merge_by_names(["id"]);
    handle.merge_arrow_batch(batch(), &merging).unwrap();
    // Both stored copies of each key update in place; merge does not turn
    // either incoming row into a third copy.
    assert_eq!(rows(&handle, &options), 4);
}
