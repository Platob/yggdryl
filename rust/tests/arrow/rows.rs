//! `rust/src/arrow/rows.rs`: the row widening no caller can name.
//!
//! `reader` is what every record I/O method hands back, so what it pulls, when
//! it pulls it, and what it answers after a row that does not convert is
//! reached through `yggdryl::internals`. Everything a caller can observe is in
//! `rust/tests/serie/value.rs` and `rust/tests/arrow/mod_.rs`.

#[cfg(feature = "internals")]
mod widening {
    use std::convert::Infallible;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use arrow_array::{Int32Array, RecordBatchReader as _};
    use arrow_schema::ArrowError;

    use yggdryl::internals::arrow_rows::reader;
    use yggdryl::{DataType, Error as CoreError, Field, Scalar, StructType};

    fn field() -> Field {
        StructType::from_fields([
            DataType::Int32.required_field("id"),
            DataType::utf8().nullable_field("name"),
        ])
        .map(DataType::from)
        .unwrap()
        .required_field("row")
    }

    #[derive(Clone)]
    struct Row {
        id: i32,
        name: Option<&'static str>,
    }

    impl From<Row> for Scalar {
        fn from(row: Row) -> Self {
            Scalar::from_sequence([
                Scalar::from(row.id),
                row.name.map_or(Scalar::Null, Scalar::from),
            ])
        }
    }

    struct Counted {
        next: usize,
        end: usize,
        pulls: Arc<AtomicUsize>,
    }

    impl Iterator for Counted {
        type Item = Row;

        fn next(&mut self) -> Option<Self::Item> {
            if self.next == self.end {
                return None;
            }
            self.pulls.fetch_add(1, Ordering::Relaxed);
            let id = self.next as i32;
            self.next += 1;
            Some(Row {
                id,
                name: Some("row"),
            })
        }
    }

    #[test]
    fn custom_structs_stream_in_bounded_batches() {
        let pulls = Arc::new(AtomicUsize::new(0));
        let rows = Counted {
            next: 0,
            end: 5,
            pulls: Arc::clone(&pulls),
        };
        let mut batches = reader(&field(), rows, Some(2), None, None, None).unwrap();
        assert_eq!(pulls.load(Ordering::Relaxed), 0);

        let first = batches.next().unwrap().unwrap();
        assert_eq!(first.num_rows(), 2);
        assert_eq!(pulls.load(Ordering::Relaxed), 2);
        assert_eq!(
            first
                .column(0)
                .as_any()
                .downcast_ref::<Int32Array>()
                .unwrap()
                .values(),
            &[0, 1]
        );
        assert_eq!(batches.next().unwrap().unwrap().num_rows(), 2);
        assert_eq!(batches.next().unwrap().unwrap().num_rows(), 1);
        assert!(batches.next().is_none());
        assert_eq!(pulls.load(Ordering::Relaxed), 5);
    }

    #[test]
    fn empty_rows_keep_the_declared_schema_without_a_pull() {
        let mut batches = reader::<_, Scalar>(&field(), [], None, None, None, None).unwrap();
        assert_eq!(batches.schema(), field().into_arrow_schema().unwrap());
        assert!(batches.next().is_none());
    }

    #[test]
    fn zero_batch_row_size_still_makes_forward_progress() {
        let rows = [Row { id: 1, name: None }, Row { id: 2, name: None }];
        let batches = reader(&field(), rows, Some(0), None, None, None).unwrap();
        assert_eq!(
            batches
                .map(|batch| batch.unwrap().num_rows())
                .sum::<usize>(),
            2
        );
    }

    #[test]
    fn an_invalid_row_follows_the_completed_batch_prefix_and_fuses_the_reader() {
        let rows = [
            Scalar::from_sequence([Scalar::from(1_i32), Scalar::from("ok")]),
            Scalar::from_sequence([Scalar::from("wrong"), Scalar::from("bad")]),
            Scalar::from_sequence([Scalar::from(3_i32), Scalar::from("unread")]),
        ];
        let mut batches = reader(&field(), rows, Some(3), None, None, None).unwrap();
        let prefix = batches.next().unwrap().unwrap();
        assert_eq!(prefix.num_rows(), 1);
        assert_eq!(
            prefix
                .column(0)
                .as_any()
                .downcast_ref::<Int32Array>()
                .unwrap()
                .values(),
            &[1]
        );
        let error = batches.next().unwrap().unwrap_err();
        let ArrowError::ExternalError(error) = error else {
            panic!("expected a typed external error")
        };
        let error = error.downcast::<CoreError>().unwrap();
        assert!(matches!(*error, CoreError::InvalidRecord { .. }));
        assert!(batches.next().is_none());
    }

    #[test]
    fn empty_struct_rows_preserve_their_row_count() {
        let root = DataType::from(StructType::from_fields([]).unwrap()).required_field("empty");
        let mut batches = reader(
            &root,
            [Scalar::from_sequence([]), Scalar::from_sequence([])],
            None,
            None,
            None,
            None,
        )
        .unwrap();
        let batch = batches.next().unwrap().unwrap();
        assert_eq!(batch.num_columns(), 0);
        assert_eq!(batch.num_rows(), 2);
    }

    #[test]
    fn infallible_into_value_uses_the_standard_try_into_path() {
        fn assert_error(_: Infallible) -> CoreError {
            unreachable!()
        }
        let _ = assert_error as fn(Infallible) -> CoreError;
        let batches = reader(
            &field(),
            [Row {
                id: 7,
                name: Some("x"),
            }],
            None,
            None,
            None,
            None,
        )
        .unwrap();
        assert_eq!(batches.count(), 1);
    }

    #[test]
    fn batches_align_to_non_divisible_commit_and_global_row_boundaries() {
        let pulls = Arc::new(AtomicUsize::new(0));
        let rows = Counted {
            next: 0,
            end: 10,
            pulls: Arc::clone(&pulls),
        };
        let mut batches = reader(&field(), rows, Some(2), None, Some(3), Some(5)).unwrap();

        assert_eq!(batches.next().unwrap().unwrap().num_rows(), 2);
        assert_eq!(batches.next().unwrap().unwrap().num_rows(), 1);
        assert_eq!(pulls.load(Ordering::Relaxed), 3);
        assert_eq!(batches.next().unwrap().unwrap().num_rows(), 2);
        assert_eq!(pulls.load(Ordering::Relaxed), 5);
        assert!(batches.next().is_none());
        assert_eq!(pulls.load(Ordering::Relaxed), 5);
    }
}

mod row_values {
    use yggdryl::{DataType, Scalar, StructType, Term};

    fn trade(id: i64, venue: Option<&str>) -> Scalar {
        Scalar::from_sequence([Scalar::from(id), venue.map_or(Scalar::Null, Scalar::from)])
    }

    #[test]
    fn rows_are_sequences_and_objects_are_records() {
        let row = trade(7, Some("XNAS"));
        assert_eq!(row.kind(), "serie");
        assert_eq!(
            row.as_sequence(),
            Some([Scalar::from(7), Scalar::from("XNAS")].as_slice())
        );

        let object =
            Scalar::from_struct([("id", Scalar::from(7)), ("venue", Scalar::from("XNAS"))])
                .unwrap();
        let json = String::from_utf8(yggdryl::json::into_bytes(&object).unwrap()).unwrap();
        assert_eq!(json, "{\"id\":7,\"venue\":\"XNAS\"}");
        assert_eq!(yggdryl::json::from_utf8(&json).unwrap(), object);
    }

    #[test]
    fn struct_expressions_evaluate_to_schema_ordered_sequences() {
        let schema = StructType::from_fields([DataType::Int64.required_field("source")])
            .map(DataType::from)
            .unwrap()
            .required_field("row");
        let bound = "struct(1 as id, 'XNAS' as venue)"
            .parse::<Term>()
            .unwrap()
            .bind(&schema)
            .unwrap();
        let source = Scalar::from_sequence([Scalar::from(0)]);
        let expected = Scalar::from_sequence([Scalar::from(1), Scalar::from("XNAS")]);
        assert_eq!(bound.eval(&source).unwrap(), expected);

        let printed = bound.term().to_string();
        assert!(printed.contains("struct("), "{printed}");
        let reparsed = printed.parse::<Term>().unwrap().bind(&schema).unwrap();
        assert_eq!(reparsed.eval(&source).unwrap(), expected);
    }

    mod arrow_bridge {

        use std::sync::Arc;
        use yggdryl::StructType;

        use arrow_array::{Int64Array, RecordBatch, StringArray};
        use yggdryl::{ArrowCastOptions, DataType, Scalar, Serie};

        fn batch() -> RecordBatch {
            let schema = StructType::from_fields([
                DataType::Int64.required_field("id"),
                DataType::utf8().nullable_field("venue"),
            ])
            .map(DataType::from)
            .unwrap()
            .required_field("row")
            .into_arrow_schema()
            .unwrap();
            RecordBatch::try_new(
                schema,
                vec![
                    Arc::new(Int64Array::from(vec![1_i64, 2])),
                    Arc::new(StringArray::from(vec![Some("XNAS"), None])),
                ],
            )
            .unwrap()
        }

        #[test]
        fn a_batch_reads_as_schema_ordered_row_sequences() {
            // The batch lands as one record column, and the column is the one
            // value its rows are.
            let value = Scalar::from(
                Serie::from_arrow_batch(None, &batch(), ArrowCastOptions::default()).unwrap(),
            );
            let rows = value.sequence_rows().expect("a sequence of rows");
            assert_eq!(rows.len(), 2);
            assert_eq!(
                rows[0].as_sequence(),
                Some([Scalar::from(1), Scalar::from("XNAS")].as_slice())
            );
            assert_eq!(rows[1].as_sequence().unwrap()[1], Scalar::Null);

            let json = String::from_utf8(yggdryl::json::into_bytes(&value).unwrap()).unwrap();
            assert_eq!(json, "[[1,\"XNAS\"],[2,null]]");
        }

        #[test]
        fn an_array_reads_as_the_sequence_its_values_spell() {
            let field = DataType::Int64.nullable_field("id");
            let array = Arc::new(Int64Array::from(vec![Some(5_i64), None]));
            let value = Scalar::from(
                Serie::from_arrow_array(Some(&field), array, ArrowCastOptions::default()).unwrap(),
            );
            assert_eq!(
                value,
                Scalar::from_sequence([Scalar::from(5), Scalar::Null])
            );
        }
    }
}
