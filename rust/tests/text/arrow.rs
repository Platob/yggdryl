//! `rust/src/text/arrow.rs`: `text/plain` rows across the Scalar/Arrow record
//! boundary, in both directions.

mod text {
    use arrow_array::StringArray;
    use yggdryl::holder::Buffer;
    use yggdryl::media::{IORecordOptions as _, RecordOptions};
    use yggdryl::text::TextOptions;
    use yggdryl::{DataType, StructType};
    use yggdryl::{IOBase as _, IOMedia as _};

    fn named(name: &str, bytes: &[u8]) -> Buffer {
        Buffer::from_bytes(bytes.to_vec()).with_media_type(
            yggdryl::Url::from_str(&format!("file:///{name}"))
                .unwrap()
                .media_type(),
        )
    }

    #[test]
    fn a_dictionary_encoded_body_is_a_text_body_a_write_unpacks_once() {
        // Arrow JS infers `Dictionary<Int32, Utf8>` for a plain record's string,
        // so a body column in that layout is the same text under another
        // spelling, unpacked once per batch rather than refused.
        use std::sync::Arc;

        use arrow_array::{DictionaryArray, Int32Array, RecordBatch};

        let mut target = named("dictionary.txt", b"old");
        let options: RecordOptions = TextOptions::new().into();
        let keys = Int32Array::from(vec![0, 1, 0]);
        let values = Arc::new(StringArray::from(vec!["one", "two"]));
        let body = DictionaryArray::<arrow_array::types::Int32Type>::try_new(keys, values).unwrap();
        let schema = Arc::new(arrow_schema::Schema::new(vec![
            arrow_schema::Field::new("sourceurl", arrow_schema::DataType::Utf8, false),
            arrow_schema::Field::new(
                "body",
                arrow_schema::DataType::Dictionary(
                    Box::new(arrow_schema::DataType::Int32),
                    Box::new(arrow_schema::DataType::Utf8),
                ),
                false,
            ),
        ]));
        let batch = RecordBatch::try_new(
            schema,
            vec![
                Arc::new(StringArray::from(vec!["input", "input", "input"])),
                Arc::new(body),
            ],
        )
        .unwrap();
        target.overwrite_arrow_batch(batch, &options).unwrap();
        assert_eq!(target.read_all_bytes().unwrap(), b"one\ntwo\none\n");
    }

    #[test]
    fn a_binary_body_is_refused_by_a_write_naming_what_it_expected() {
        // A text row's body is text, and a column that may hold anything is not
        // written as if it were: the refusal names the column and the layout.
        let mut target = named("refused.txt", b"old");
        let mut options: RecordOptions = TextOptions::new().into();
        let field = StructType::from_fields([
            DataType::utf8().required_field("sourceurl"),
            DataType::binary().required_field("body"),
        ])
        .map(DataType::from)
        .unwrap()
        .required_field("row");
        options.set_field(field);
        let rows = [yggdryl::Scalar::from_struct([
            ("sourceurl", yggdryl::Scalar::from("input")),
            ("body", yggdryl::Scalar::from(&b"first"[..])),
        ])
        .unwrap()];
        let error = target
            .overwrite_records(rows, &options)
            .unwrap_err()
            .to_string();
        assert!(
            error.contains("expected a utf8 body column, got Binary"),
            "{error}"
        );
        assert_eq!(target.read_all_bytes().unwrap(), b"old");
    }

    // --- Reading Arrow back into lines ---

    mod intake {

        use yggdryl::FieldPath;
        use yggdryl::graph::Event as _;
        use yggdryl::text::TextOptions;
        use yggdryl::text::{from_arrow_batch, from_arrow_reader};
        use yggdryl::text::{into_arrow_batch, read_text_lines};

        use super::named;

        fn decode(source: &[u8], options: &TextOptions) -> Vec<yggdryl::text::TextLine> {
            read_text_lines(&named("app.log", source), options)
                .expect("a settled configuration")
                .map(|line| line.expect("a line"))
                .collect()
        }

        #[test]
        fn a_batch_round_trips_back_into_its_lines() {
            let mut options = TextOptions::new();
            options.start_rownum = Some(0);
            options.parse_mimetype = true;
            let lines = decode(b"first\nsecond\n", &options);
            let batch = into_arrow_batch(lines.clone(), &options).expect("a batch");
            let back = from_arrow_batch(&batch, &options).expect("lines read back");

            assert_eq!(back.len(), lines.len());
            for (read, original) in back.iter().zip(&lines) {
                assert_eq!(read.body(), original.body());
                assert_eq!(read.index(), original.index());
                assert_eq!(
                    read.sourceurl().map(ToString::to_string),
                    original.sourceurl().map(ToString::to_string)
                );
                assert_eq!(read.mtime().unwrap(), original.mtime().unwrap());
            }
        }

        #[test]
        fn a_column_named_the_way_someone_else_writes_it_is_still_found() {
            let options = TextOptions::new();
            // A producer that calls the body `payload` and the object `source`.
            let renamed = TextOptions::new()
                .with_renamed_column("body", "payload")
                .with_renamed_column("sourceurl", "source");
            let lines = decode(b"hello\n", &renamed);
            let foreign = into_arrow_batch(lines, &renamed).expect("a batch");

            // Read under the ordinary names: the aliases carry it.
            let back = from_arrow_batch(&foreign, &options).expect("lines read back");
            assert_eq!(back.len(), 1);
            assert_eq!(back[0].body(), "hello");
            assert!(back[0].sourceurl().is_some(), "source resolved to url");
        }

        #[test]
        fn a_case_difference_alone_still_matches() {
            let renamed = TextOptions::new().with_renamed_column("body", "BODY");
            let lines = decode(b"hello\n", &renamed);
            let batch = into_arrow_batch(lines, &renamed).expect("a batch");
            let back = from_arrow_batch(&batch, &TextOptions::new()).expect("lines read back");
            assert_eq!(back[0].body(), "hello");
        }

        #[test]
        fn a_column_the_batch_does_not_carry_leaves_its_field_at_the_default() {
            let mut with_rownum = TextOptions::new();
            with_rownum.start_rownum = Some(5);
            let lines = decode(b"only\n", &TextOptions::new());
            let batch = into_arrow_batch(lines, &TextOptions::new()).expect("a batch");
            // The reading options want a rownum column the batch has none of.
            let back = from_arrow_batch(&batch, &with_rownum).expect("lines read back");
            assert_eq!(back.len(), 1);
            assert_eq!(back[0].index(), 0, "the position it was read at");
            assert_eq!(back[0].body(), "only");
        }

        #[test]
        fn a_lifted_column_round_trips_back_into_its_entry() {
            let options = TextOptions::new()
                .try_with_lift_names(["55"])
                .expect("the path parses");
            let lines = decode(b"55=AAPL\n", &options);
            let batch = into_arrow_batch(lines, &options).expect("a batch");
            let back = from_arrow_batch(&batch, &options).expect("lines read back");
            let path = FieldPath::from_str("55").expect("the path parses");
            assert_eq!(
                back[0]
                    .get_entry_by_path(&path)
                    .and_then(|held| held.value_bytes().as_str()),
                Some("AAPL")
            );
        }

        #[test]
        fn streamed_batches_read_back_one_at_a_time() {
            let mut options = TextOptions::new();
            options.batch_row_size = Some(1);
            let lines = decode(b"a\nb\nc\n", &options);
            let reader = yggdryl::text::into_arrow_reader(
                lines.into_iter().map(Ok).collect::<Vec<_>>(),
                &options,
            )
            .expect("a reader");
            let back: Vec<_> = from_arrow_reader(reader, &options)
                .expect("a settled configuration")
                .map(|line| line.expect("a line"))
                .collect();
            assert_eq!(back.len(), 3);
            assert_eq!(back[2].body(), "c");
        }

        #[test]
        fn a_stated_instant_is_the_rows_word_over_the_headers() {
            // The mtime column states the instant a line read back answers, over
            // whatever its own header would read: the row's word is the fact.
            let options = TextOptions::new()
                .try_with_rowheader(r"^(?<mtime>\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}Z) ")
                .expect("a header");
            let lines = decode(b"2026-01-02T10:15:30Z body\n", &options);
            let batch = into_arrow_batch(lines.clone(), &options).expect("a batch");
            let back = from_arrow_batch(&batch, &options).expect("lines read back");
            assert_eq!(back[0].mtime().unwrap(), lines[0].mtime().unwrap());
            assert_eq!(back[0].body(), lines[0].body());
            let mut restated = back[0].clone();
            restated.set_currunix(7);
            assert_eq!(restated.mtime().unwrap(), Some(7));
            assert_eq!(restated.get_currunix(), 7);
        }
    }
}
