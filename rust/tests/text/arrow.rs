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
    fn a_member_of_a_located_archive_reads_its_own_lines_not_the_archive_s() {
        // A member is addressed in the archive's URL fragment, so the file name
        // of its location is the archive's: reopening the member by that name
        // would read the archive's own bytes as the lines. The member reads as
        // the lines it holds, from an archive that has a location.
        use arrow_array::Array as _;

        let mut root = yggdryl::local::LocalFolder::temporary()
            .unwrap()
            .path()
            .unwrap();
        root.push(format!("yggdryl-text-zip-member-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        let archive =
            yggdryl::zip::mount(yggdryl::holder::Holder::local(root.join("day.zip")).unwrap());
        let mut member = archive.child_by_path("notes/day.txt").unwrap();
        member.write_all_bytes(b"one\ntwo\n").unwrap();
        let member = archive.child_by_path("notes/day.txt").unwrap();
        let options: RecordOptions = TextOptions::new().into();
        let batches: Vec<arrow_array::RecordBatch> = member
            .read_arrow_reader(&options)
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        let bodies: Vec<String> = batches
            .iter()
            .flat_map(|batch| {
                let body = batch
                    .column_by_name("body")
                    .expect("a body column")
                    .as_any()
                    .downcast_ref::<StringArray>()
                    .expect("text")
                    .clone();
                (0..body.len()).map(move |row| body.value(row).to_owned())
            })
            .collect();
        assert_eq!(bodies, ["one", "two"]);
        let _ = std::fs::remove_dir_all(&root);
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
            arrow_schema::Field::new("crosscode", arrow_schema::DataType::Utf8, false),
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
            DataType::utf8().required_field("crosscode"),
            DataType::binary().required_field("body"),
        ])
        .map(DataType::from)
        .unwrap()
        .required_field("row");
        options.set_field(field);
        let rows = [yggdryl::Scalar::from_struct([
            ("crosscode", yggdryl::Scalar::from("input")),
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

    #[test]
    fn a_code_body_laid_out_as_utf8_is_still_refused_by_the_body_reader() {
        // A currency is stored as UTF-8, so its layout passes the schema; the
        // column lands as a currency, not as text, and is refused by its type.
        let mut target = named("coded.txt", b"old");
        let mut options: RecordOptions = TextOptions::new().into();
        let field = StructType::from_fields([
            DataType::utf8().required_field("crosscode"),
            DataType::Ccy.required_field("body"),
        ])
        .map(DataType::from)
        .unwrap()
        .required_field("row");
        options.set_field(field);
        let rows = [yggdryl::Scalar::from_struct([
            ("crosscode", yggdryl::Scalar::from("input")),
            (
                "body",
                yggdryl::Scalar::Ccy(yggdryl::Ccy::new("EUR").unwrap()),
            ),
        ])
        .unwrap()];
        let error = target
            .overwrite_records(rows, &options)
            .unwrap_err()
            .to_string();
        assert!(
            error.contains("expected a utf8 body column, got ccy"),
            "{error}"
        );
        assert_eq!(target.read_all_bytes().unwrap(), b"old");
    }

    #[test]
    fn every_text_layout_a_body_may_take_writes_its_lines() {
        // Each lands in a text storage leaf of its own - offsets, large
        // offsets, views, and an ASCII leaf laid out as UTF-8 - and every one
        // is read as the body.
        for body in [
            DataType::utf8(),
            DataType::large_utf8(),
            DataType::utf8_view(),
            DataType::ascii(),
        ] {
            let mut target = named("layouts.txt", b"old");
            let mut options: RecordOptions = TextOptions::new().into();
            let field = StructType::from_fields([
                DataType::utf8().required_field("crosscode"),
                body.clone().required_field("body"),
            ])
            .map(DataType::from)
            .unwrap()
            .required_field("row");
            options.set_field(field);
            let rows = ["one", "two"].map(|text| {
                yggdryl::Scalar::from_struct([
                    ("crosscode", yggdryl::Scalar::from("input")),
                    ("body", yggdryl::Scalar::from(text)),
                ])
                .unwrap()
            });
            target
                .overwrite_records(rows, &options)
                .unwrap_or_else(|error| panic!("{body}: {error}"));
            assert_eq!(target.read_all_bytes().unwrap(), b"one\ntwo\n", "{body}");
        }
    }

    // --- Reading Arrow back into lines ---

    mod intake {

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
            // The two facts a line no longer has a column of its own for are
            // read back off the event columns that state them: the object out
            // of `crosscode`, the place out of `seqnum`.
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
            // The object a line came from is the `crosscode` column now, so
            // that is the column the other spelling renames - and the same
            // spelling resolves back onto it on the way in.
            let renamed = TextOptions::new()
                .with_renamed_column("body", "payload")
                .with_renamed_column("crosscode", "source");
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
        fn a_column_that_states_nothing_leaves_its_field_at_the_default() {
            let mut with_offset = TextOptions::new();
            with_offset.start_rownum = Some(5);
            // Written with no offset, so the first line has no place in a
            // chain and its `seqnum` cell is null - a place is a count, and
            // `seqnum` is null at zero.
            let lines = decode(b"only\n", &TextOptions::new());
            let batch = into_arrow_batch(lines, &TextOptions::new()).expect("a batch");
            // The reading options count from five. Nothing stated is nothing
            // to take an offset off, so the line keeps the position the
            // stream read it at rather than a place the row never stated.
            let back = from_arrow_batch(&batch, &with_offset).expect("lines read back");
            assert_eq!(back.len(), 1);
            assert_eq!(back[0].index(), 0, "the position it was read at");
            assert_eq!(back[0].body(), "only");
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
            // `currunix` states the instant a line read back answers, over
            // whatever its own header would read: the row's word is the fact.
            // With `parse_mtime` on the capture dates the line and has no
            // column beside it - one owner per fact.
            let options = TextOptions::new()
                .try_with_rowheader(r"^(?<mtime>\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}Z) ")
                .expect("a header");
            let lines = decode(b"2026-01-02T10:15:30Z body\n", &options);
            let batch = into_arrow_batch(lines.clone(), &options).expect("a batch");
            assert!(batch.schema().index_of("mtime").is_err(), "no mtime column");
            assert!(batch.schema().index_of("currunix").is_ok());
            let back = from_arrow_batch(&batch, &options).expect("lines read back");
            assert_eq!(back[0].mtime().unwrap(), lines[0].mtime().unwrap());
            assert_eq!(back[0].body(), lines[0].body());
            let mut restated = back[0].clone();
            restated.set_currunix(7);
            assert_eq!(restated.mtime().unwrap(), Some(7));
            assert_eq!(restated.get_currunix(), 7);
        }
    }

    // --- The row header the cut reads ---

    mod header {
        use std::sync::Arc;

        use yggdryl::text::{TextBytes, TextLine, TextOptions, read_text_lines};

        use super::named;

        /// The bodies and captures a read states, line by line.
        fn read(source: &[u8], options: &TextOptions) -> Vec<(String, Vec<Option<String>>)> {
            read_text_lines(&named("app.log", source), options)
                .expect("a settled configuration")
                .map(|line| stated(&line.expect("a line")))
                .collect()
        }

        /// What the line built from `bytes` alone states: its own match.
        fn own(bytes: &[u8], options: &TextOptions) -> (String, Vec<Option<String>>) {
            let body = TextBytes::from_bytes(bytes).expect("a page");
            stated(&TextLine::from_bytes(0, body, Arc::new(options.clone())).expect("a line"))
        }

        fn stated(line: &TextLine) -> (String, Vec<Option<String>>) {
            let captures = (0..line.captures().len())
                .map(|at| line.capture(at).map(str::to_owned))
                .collect();
            (line.body().to_owned(), captures)
        }

        #[test]
        fn a_line_the_header_does_not_match_keeps_its_body_and_captures_nothing() {
            // The reader matches the header where the line would, so a line
            // it does not match reads exactly as the line's own match
            // answers: the body whole and every capture empty, beside lines
            // that match with and without their optional group.
            let options = TextOptions::new()
                .try_with_rowheader(r"^\[(?<level>[A-Z]+)\] (?:(?<code>\d+) )?")
                .expect("a header");
            let source = b"[WARN] 7 first\nsecond [INFO]\n[INFO] third\n";
            let lines = read(source, &options);
            assert_eq!(
                lines,
                [
                    (
                        "first".to_owned(),
                        vec![Some("WARN".to_owned()), Some("7".to_owned())]
                    ),
                    ("second [INFO]".to_owned(), vec![None, None]),
                    ("third".to_owned(), vec![Some("INFO".to_owned()), None]),
                ]
            );
            for (line, bytes) in lines.iter().zip(source.split(|byte| *byte == b'\n')) {
                assert_eq!(*line, own(bytes, &options));
            }
        }

        #[test]
        fn a_leading_fragment_is_matched_over_the_record_it_opens() {
            // A framed record joins the lines behind the one it opens with,
            // so a fragment no single line matches is matched over the whole
            // record it became - the joined lines here - never over its first
            // line alone.
            let options = TextOptions::new()
                .try_with_rowheader(r"^(?<first>[a-z]+)\s+(?<second>[a-z]+) ")
                .expect("a header")
                .with_framing(true);
            assert_eq!(
                read(b"abc\ndef ghi\n", &options),
                [own(b"abc\ndef ghi", &options)]
            );
            assert_eq!(
                read(b"abc\ndef ghi\n", &options),
                [(
                    "ghi".to_owned(),
                    vec![Some("abc".to_owned()), Some("def".to_owned())]
                )]
            );
        }
    }

    // --- The clauses and the one decode ---

    mod clauses {

        use arrow_array::StringArray;
        use yggdryl::IOMedia as _;
        use yggdryl::media::IORecordOptions as _;
        use yggdryl::text::{TextOptions, read_text_lines};

        use super::named;

        const SOURCE: &[u8] = b"alpha\nbravo\ncharlie\ndelta\n";

        /// Every body the one decode yields, in order.
        fn decoded(source: &[u8], options: &TextOptions) -> Vec<String> {
            read_text_lines(&named("app.log", source), options)
                .expect("a settled configuration")
                .map(|line| line.expect("a line").body().to_owned())
                .collect()
        }

        /// Every value of one published column, in order.
        fn published(source: &[u8], options: &TextOptions, column: &str) -> Vec<String> {
            named("app.log", source)
                .read_arrow_reader(&options.clone().into())
                .expect("a reader")
                .map(|batch| batch.expect("a batch"))
                .flat_map(|batch| {
                    let index = batch.schema().index_of(column).expect("the column");
                    batch
                        .column(index)
                        .as_any()
                        .downcast_ref::<StringArray>()
                        .expect("a text column")
                        .iter()
                        .map(|value| value.expect("a value").to_owned())
                        .collect::<Vec<_>>()
                })
                .collect()
        }

        #[test]
        fn the_one_decode_yields_every_line_and_the_where_clause_keeps_rows_above_it() {
            // The decode is not the query. `read_text_lines` is the one parse
            // both surfaces route through, so it answers every line the object
            // holds; the `where` clause is a record clause, and the record
            // surface is where it keeps rows. A caller holding the iterator
            // holds the lines, not the result.
            let options = TextOptions::new()
                .with_filter("body like 'b%'")
                .expect("a clause");

            assert_eq!(
                decoded(SOURCE, &options),
                ["alpha", "bravo", "charlie", "delta"]
            );
            assert_eq!(published(SOURCE, &options, "body"), ["bravo"]);
        }

        #[test]
        fn a_where_clause_naming_a_projection_alias_cannot_be_read_at_the_line() {
            // Why the clause belongs above the decode rather than inside it: a
            // `where` may name what the `select` made, and no line states a
            // name the projection has not built yet. The clause runs after the
            // projection here, and there is nothing at the line to run.
            let options = TextOptions::new()
                .with_select("trim(body) as line")
                .expect("a projection")
                .with_filter("line like 'b%'")
                .expect("a clause");

            assert_eq!(
                decoded(SOURCE, &options),
                ["alpha", "bravo", "charlie", "delta"]
            );
            assert_eq!(published(SOURCE, &options, "line"), ["bravo"]);
        }

        #[test]
        fn a_where_clause_reads_every_column_the_row_schema_states() {
            // The clause binds against the whole row a line becomes - the
            // seventeen event columns it opens with, the place they state, and
            // the header's own captures - not against the body alone.
            let mut options = TextOptions::new()
                .try_with_rowheader(r"^\[(?<level>[A-Z]+)\] ")
                .expect("a header");
            options.start_rownum = Some(1);
            let options = options
                .with_filter("level = 'WARN' and seqnum > 1")
                .expect("a clause");
            let source = b"[WARN] first\n[INFO] second\n[WARN] third\n";

            // Every body is the line past its row header: the level the
            // header lifted into its own column is off the body.
            assert_eq!(decoded(source, &options), ["first", "second", "third"]);
            assert_eq!(published(source, &options, "body"), ["third"]);
        }

        #[test]
        fn a_where_clause_naming_no_column_is_refused_by_the_read_and_not_by_the_decode() {
            // The row schema states no `rownum` under any configuration - a
            // line's place is its `seqnum` - so the read refuses the clause by
            // name, before a byte is pulled; the decode never binds it at all,
            // because the decode is not the query.
            let options = TextOptions::new()
                .with_filter("rownum > 1")
                .expect("a clause");

            assert_eq!(
                decoded(SOURCE, &options),
                ["alpha", "bravo", "charlie", "delta"]
            );
            let error = match named("app.log", SOURCE).read_arrow_reader(&options.into()) {
                Err(error) => error.to_string(),
                Ok(_) => panic!("a column the schema does not state is refused"),
            };
            assert!(error.contains("rownum"), "{error}");
        }
    }
}
