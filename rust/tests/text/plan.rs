//! `rust/src/text/plan.rs`: the compiled column plan one text read is answered
//! by - which columns exist, under which names, at which datatypes.

mod columns {
    use arrow_array::{Array as _, StringArray, UInt64Array};
    use yggdryl::holder::Buffer;
    use yggdryl::media::{IORecordOptions as _, RecordOptions};
    use yggdryl::text::{Text, TextOptions};
    use yggdryl::{Codec, DataType, Field, Timezone};
    use yggdryl::{IOBase as _, IOMedia as _};

    /// One hash of any hashable value, for "equal values hash alike" and for a
    /// temporary name that does not collide. The crate's own stable hash is
    /// private, and neither use needs it to be stable across runs.
    pub(super) fn hash_of<T: std::hash::Hash>(value: &T) -> u64 {
        use std::hash::Hasher as _;

        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        value.hash(&mut hasher);
        hasher.finish()
    }

    fn named(name: &str, bytes: &[u8]) -> Buffer {
        Buffer::from_bytes(bytes.to_vec()).with_media_type(
            yggdryl::Url::from_str(&format!("file:///{name}"))
                .unwrap()
                .media_type(),
        )
    }

    fn options(rowheader: &str) -> TextOptions {
        TextOptions::new().try_with_rowheader(rowheader).unwrap()
    }

    fn framed(rowheader: &str) -> TextOptions {
        options(rowheader).with_framing(true)
    }

    /// The nineteen event columns every line batch opens with, in front of the
    /// line's own: the line is an event of the graph, and a message parsed out
    /// of it contains the same nineteen under the same names and datatypes.
    const EVENT_COLUMNS: [&str; 19] = [
        "currunix",
        "creaunix",
        "execunix",
        "recdunix",
        "refrecdunix",
        "exprtime",
        "prevunix",
        "snapunix",
        "curruuid",
        "crossuuid",
        "crosscode",
        "currhashcode",
        "crosshashcode",
        "prevuuid",
        "seqnum",
        "parentuuids",
        "srcuuids",
        "identifiers",
        "state",
    ];

    /// The names of a line batch: the event columns, then the line's own.
    fn with_event(rest: &[&'static str]) -> Vec<&'static str> {
        EVENT_COLUMNS.iter().chain(rest).copied().collect()
    }

    fn collect(
        source: &impl yggdryl::IOBase,
        options: TextOptions,
    ) -> Vec<arrow_array::RecordBatch> {
        source
            .read_arrow_reader(&options.into())
            .unwrap()
            .collect::<std::result::Result<Vec<_>, _>>()
            .unwrap()
    }

    fn bodies(batches: &[arrow_array::RecordBatch]) -> Vec<Vec<u8>> {
        batches
            .iter()
            .flat_map(|batch| {
                let index = batch.schema().index_of("body").unwrap();
                batch
                    .column(index)
                    .as_any()
                    .downcast_ref::<StringArray>()
                    .unwrap()
                    .iter()
                    .map(|value| value.unwrap().as_bytes().to_vec())
                    .collect::<Vec<_>>()
            })
            .collect()
    }

    /// One text column.
    fn texts<'a>(batches: &'a [arrow_array::RecordBatch], name: &str) -> Vec<Option<&'a str>> {
        batches
            .iter()
            .flat_map(|batch| {
                let index = batch.schema().index_of(name).unwrap();
                batch
                    .column(index)
                    .as_any()
                    .downcast_ref::<StringArray>()
                    .unwrap()
                    .iter()
                    .collect::<Vec<_>>()
            })
            .collect()
    }

    #[test]
    fn ordinary_record_reading_numbers_the_rows_in_seqnum_and_types_captures_by_regex() {
        let source = named(
            "app.log",
            b"  [INFO] id=7 first  \r\n[WARN] id=9 second\nplain\r",
        );
        let mut text = options(r"\[(?<level>[A-Z]+)\] id=(?<id>\d+)");
        text.start_rownum = Some(10);
        text.set_lstrip(Some(r"^\s+")).unwrap();
        text.set_rstrip(Some(r"\s+$")).unwrap();
        let options = text.into();

        let batches = source
            .read_arrow_reader(&options)
            .unwrap()
            .collect::<std::result::Result<Vec<_>, _>>()
            .unwrap();
        assert_eq!(batches.len(), 1);
        let batch = &batches[0];
        // Nothing stands between the event columns and the line's own: the
        // row number is the event's place, `seqnum`, and the object the line
        // came from is the event's chain, `crosscode`.
        assert_eq!(batch.schema().field(14).name(), "seqnum");
        assert_eq!(batch.schema().field(19).name(), "body");
        assert_eq!(
            batch.schema().field(21).data_type(),
            &arrow_schema::DataType::Int64
        );
        assert_eq!(
            batch
                .column(14)
                .as_any()
                .downcast_ref::<UInt64Array>()
                .unwrap()
                .iter()
                .collect::<Vec<_>>(),
            [Some(10), Some(11), Some(12)]
        );
        // The body is the line past its row header - the edges stripped, what
        // the header matched taken off - and the captures state what it took.
        assert_eq!(
            batch
                .column(19)
                .as_any()
                .downcast_ref::<StringArray>()
                .unwrap()
                .iter()
                .collect::<Vec<_>>(),
            [Some(" first"), Some(" second"), Some("plain")]
        );
        assert_eq!(
            batch
                .column(20)
                .as_any()
                .downcast_ref::<StringArray>()
                .unwrap()
                .iter()
                .collect::<Vec<_>>(),
            [Some("INFO"), Some("WARN"), None]
        );
    }

    #[test]
    fn seqnum_is_null_where_the_line_has_no_place_in_its_chain() {
        // Every read answers `seqnum`, numbered or not. Asked for no first row
        // number, the place is the physical line number, and the first line's
        // zero is no place at all: the column counts what came before a line,
        // and before the first line nothing did.
        let source = named("rows.log", b"first\nsecond\nthird\n");
        let batch = collect(&source, TextOptions::new()).pop().unwrap();
        assert_eq!(
            batch.schema().field(14).data_type(),
            &arrow_schema::DataType::UInt64
        );
        assert!(batch.schema().field(14).is_nullable());
        assert_eq!(
            batch
                .column(14)
                .as_any()
                .downcast_ref::<UInt64Array>()
                .unwrap()
                .iter()
                .collect::<Vec<_>>(),
            [None, Some(1), Some(2)]
        );

        // Asked for one, the first row number is the first line's place and
        // the count runs from there, so no line of a numbered read is placeless.
        let mut numbered = TextOptions::new();
        numbered.start_rownum = Some(1);
        let batch = collect(&source, numbered).pop().unwrap();
        assert_eq!(
            batch
                .column(14)
                .as_any()
                .downcast_ref::<UInt64Array>()
                .unwrap()
                .iter()
                .collect::<Vec<_>>(),
            [Some(1), Some(2), Some(3)]
        );
    }

    #[test]
    fn capture_schema_is_derived_from_regex_before_reading() {
        let mut strings = options(r"(?<value>\d+)");
        strings.set_autotype(false);
        let field = named("empty.log", b"")
            .read_arrow_field(&strings.into())
            .unwrap();
        assert_eq!(field.field("value").unwrap().dtype(), &DataType::utf8(),);

        let typed = options(r"(?<value>\d+)");
        let field = named("empty.log", b"")
            .read_arrow_field(&typed.clone().into())
            .unwrap();
        assert_eq!(field.field("value").unwrap().dtype(), &DataType::Int64);

        let batch = named("values.log", b"1\n2\n")
            .read_arrow_reader(&typed.into())
            .unwrap()
            .next()
            .unwrap()
            .unwrap();
        assert_eq!(
            batch.schema().field(20).data_type(),
            &arrow_schema::DataType::Int64
        );

        let broad = options(r"(?<value>\S+)");
        let batch = named("values.log", b"1\nword\n")
            .read_arrow_reader(&broad.into())
            .unwrap()
            .next()
            .unwrap()
            .unwrap();
        assert_eq!(
            batch.schema().field(20).data_type(),
            &arrow_schema::DataType::Utf8
        );
        assert_eq!(
            batch
                .schema()
                .fields()
                .iter()
                .map(|field| field.name().as_str())
                .collect::<Vec<_>>(),
            with_event(&["body", "value"])
        );
    }

    #[test]
    fn sequence_numbers_start_at_the_requested_row_number_and_refuse_what_no_count_holds() {
        let mut options = TextOptions::new();
        options.start_rownum = Some(i64::MAX);
        options.set_batch_row_size(Some(1));
        let mut reader = named("rows.log", b"first\nsecond\n")
            .read_arrow_reader(&options.into())
            .unwrap();

        let first = reader.next().unwrap().unwrap();
        assert_eq!(
            first
                .column(14)
                .as_any()
                .downcast_ref::<UInt64Array>()
                .unwrap()
                .value(0),
            u64::try_from(i64::MAX).unwrap()
        );
        let error = reader.next().unwrap().unwrap_err().to_string();
        assert!(error.contains("text row number exceeds i64::MAX"));

        // The place in a chain is a count, so a row number below zero is no
        // place at all: the read refuses it by name rather than numbering a
        // line backwards.
        let mut below = TextOptions::new();
        below.start_rownum = Some(-1);
        let error = named("rows.log", b"first\n")
            .read_arrow_reader(&below.into())
            .unwrap()
            .next()
            .unwrap()
            .unwrap_err()
            .to_string();
        assert!(error.contains("a row number a count can hold"), "{error}");
    }

    #[test]
    fn the_cross_code_is_rendered_from_the_handlers_real_url() {
        use yggdryl::local::{LocalFile, LocalFolder};

        let mut path = LocalFolder::temporary().unwrap().path().unwrap();
        path.push(format!("yggdryl-text-url-{}.log", std::process::id()));
        let mut source = LocalFile::new(&path).unwrap();
        source.remove(false).unwrap();
        source.write_all_bytes(b"body\n").unwrap();
        let expected = source.url().unwrap().to_string();

        let batch = source
            .read_arrow_reader(&TextOptions::new().into())
            .unwrap()
            .next()
            .unwrap()
            .unwrap();
        // The object a line was read from is the chain it belongs to, so the
        // URL is stated under `crosscode` and nowhere beside it.
        assert_eq!(
            batch
                .column(10)
                .as_any()
                .downcast_ref::<StringArray>()
                .unwrap()
                .value(0),
            expected
        );

        source.remove(false).unwrap();
    }

    #[test]
    fn autotyping_reads_a_session_clock_past_the_end_of_its_day() {
        // Extended session hours spell the small hours as 24 and up; the capture
        // is a time of day, so each reading folds into its day.
        let batch = named("session.log", b"08:00:00\n25:30:00\n")
            .read_arrow_reader(&options(r"(?<clock>\d{2}:\d{2}:\d{2})").into())
            .unwrap()
            .next()
            .unwrap()
            .unwrap();

        assert_eq!(
            batch.schema().field(20).data_type(),
            &arrow_schema::DataType::Time32(arrow_schema::TimeUnit::Second)
        );
        assert_eq!(
            batch
                .column(20)
                .as_any()
                .downcast_ref::<arrow_array::Time32SecondArray>()
                .unwrap()
                .iter()
                .collect::<Vec<_>>(),
            [Some(28_800), Some(5_400)]
        );
    }

    #[test]
    fn a_real_log_row_captures_a_microsecond_timestamp_and_binary_body() {
        let source = named(
            "execution.log",
            b"2026-08-29 00:00:00.434_958 [77-2f3e6ff7:9f4d2a08b1:128] \
[ModuleFailFastFilterChecker] (DEBUG) Execution report \
(execId: 20260828180000369318, from session:\n",
        );
        let text = TextOptions::new()
            .try_with_rowheader(concat!(
                r"^(?<stamp>\d{4}-\d{2}-\d{2} \d{2}:\d{2}:\d{2}\.\d{3}_\d{3}) ",
                r"\[(?<thread>[^]]+)\] \[(?<module>[^]]+)\] \((?<level>[A-Z]+)\) ",
            ))
            .unwrap();
        let mut options: RecordOptions = text.into();
        options.set_timezone(Some(Timezone::UTC)).unwrap();

        let batch = source
            .read_arrow_reader(&options)
            .unwrap()
            .next()
            .unwrap()
            .unwrap();
        assert_eq!(
            batch.schema().field(20).data_type(),
            &arrow_schema::DataType::Timestamp(
                arrow_schema::TimeUnit::Microsecond,
                Some("UTC".into())
            )
        );
        // The stamp, the thread, the module and the level are the header's,
        // so the body is what the line says after them.
        assert_eq!(
            batch
                .column(19)
                .as_any()
                .downcast_ref::<StringArray>()
                .unwrap()
                .value(0),
            "Execution report (execId: 20260828180000369318, from session:"
        );
        assert_eq!(
            batch
                .column(21)
                .as_any()
                .downcast_ref::<StringArray>()
                .unwrap()
                .value(0),
            "77-2f3e6ff7:9f4d2a08b1:128"
        );
        assert_eq!(
            batch
                .column(22)
                .as_any()
                .downcast_ref::<StringArray>()
                .unwrap()
                .value(0),
            "ModuleFailFastFilterChecker"
        );
    }

    #[test]
    fn a_comma_fraction_and_a_variable_width_one_are_timestamp_columns() {
        // ISO 8601 names the comma a decimal sign, so a log4j clock is a clock:
        // the capture's own syntax types the column at the width it spells, and
        // the row reads through the header's own match rather than through a cast
        // that would demand a zone the line never carries.
        let source = named("log4j.log", b"2026-08-14 00:05:01,148 [main] started\n");
        let text = TextOptions::new()
            .try_with_rowheader(
                r"^(?<stamp>\d{4}-\d{2}-\d{2} \d{2}:\d{2}:\d{2},\d{3}) \[(?<thread>[^]]+)\] ",
            )
            .unwrap();
        let mut options: RecordOptions = text.into();
        options.set_timezone(Some(Timezone::UTC)).unwrap();

        let batch = source
            .read_arrow_reader(&options)
            .unwrap()
            .next()
            .unwrap()
            .unwrap();
        assert_eq!(
            batch.schema().field(20).data_type(),
            &arrow_schema::DataType::Timestamp(
                arrow_schema::TimeUnit::Millisecond,
                Some("UTC".into())
            )
        );
        assert_eq!(
            batch
                .column(20)
                .as_any()
                .downcast_ref::<arrow_array::TimestampMillisecondArray>()
                .unwrap()
                .value(0),
            1_786_665_901_148
        );

        // A capture admitting several widths publishes the widest, and a row
        // spelling fewer digits restates into it exactly.
        let source = named(
            "variable.log",
            b"2026-08-14 00:05:01.148 short\n2026-08-14 00:05:01.12345 long\n",
        );
        let text = TextOptions::new()
            .try_with_rowheader(r"^(?<stamp>\d{4}-\d{2}-\d{2} \d{2}:\d{2}:\d{2}\.\d{1,5}) ")
            .unwrap();
        let mut options: RecordOptions = text.into();
        options.set_timezone(Some(Timezone::UTC)).unwrap();

        let batch = source
            .read_arrow_reader(&options)
            .unwrap()
            .next()
            .unwrap()
            .unwrap();
        assert_eq!(
            batch.schema().field(20).data_type(),
            &arrow_schema::DataType::Timestamp(
                arrow_schema::TimeUnit::Microsecond,
                Some("UTC".into())
            )
        );
        assert_eq!(
            batch
                .column(20)
                .as_any()
                .downcast_ref::<arrow_array::TimestampMicrosecondArray>()
                .unwrap()
                .iter()
                .collect::<Vec<_>>(),
            [Some(1_786_665_901_148_000), Some(1_786_665_901_123_450)]
        );
    }

    #[test]
    fn framed_schema_is_complete_before_empty_or_absent_input_is_pulled() {
        use yggdryl::local::LocalFile;

        let options = framed(r"^\[(?<kind>[A-Z])\] ").with_max_record_byte_size(10);
        let record_options: RecordOptions = options.clone().into();
        let empty = named("empty.log.gz", &Codec::Gzip.dump(b"").unwrap());
        let reader = empty.read_arrow_reader(&record_options).unwrap();
        assert_eq!(
            reader
                .schema()
                .fields()
                .iter()
                .map(|field| field.name().as_str())
                .collect::<Vec<_>>(),
            with_event(&["body", "dropped_byte_size", "kind"])
        );
        assert!(
            reader
                .collect::<std::result::Result<Vec<_>, _>>()
                .unwrap()
                .is_empty()
        );

        for suffix in ["log.gz", "log.zst"] {
            let mut path = std::env::temp_dir();
            path.push(format!(
                "yggdryl-absent-framed-schema-{}-{}.{}",
                std::process::id(),
                hash_of(&path),
                suffix
            ));
            let mut absent = LocalFile::new(&path).unwrap();
            absent.remove(false).unwrap();
            let reader = absent.read_arrow_reader(&options.clone().into()).unwrap();
            assert_eq!(
                reader
                    .schema()
                    .fields()
                    .iter()
                    .map(|field| field.name().as_str())
                    .collect::<Vec<_>>(),
                with_event(&["body", "dropped_byte_size", "kind"])
            );
            assert!(
                reader
                    .collect::<std::result::Result<Vec<_>, _>>()
                    .unwrap()
                    .is_empty()
            );
            assert_eq!(
                Text::new(absent)
                    .with_options(options.clone())
                    .row_size()
                    .unwrap(),
                0
            );
        }
    }

    #[test]
    fn the_classification_column_reads_the_line_and_leaves_the_body() {
        let source = Buffer::from_bytes(
            [
                b"sending >> 8=FIX.4.2|9=176|35=D|10=203|\n".as_slice(),
                b"recv ACCOUNT=A1|MSGTYPE=8|SYMBOL=AAPL\n".as_slice(),
                b"level=INFO worker=3 took=12ms\n".as_slice(),
                b"<Order ClOrdID='XML-1'/>\n".as_slice(),
                b"no level printed by this plugin\n".as_slice(),
            ]
            .concat(),
        );
        let mut options = TextOptions::new();
        options.parse_mimetype = true;

        // The columns a classifying read declares, in order. A direction is
        // FIX's fact and not the reader's: no column carries one.
        let field = options.source_field().unwrap();
        let names: Vec<&str> = field
            .dtype()
            .as_fields()
            .unwrap()
            .iter()
            .map(Field::name)
            .collect();
        assert_eq!(names, with_event(&["mimetype", "body"]));

        let batches = collect(&source, options);
        assert_eq!(
            texts(&batches, "mimetype"),
            [
                Some("text/fix"),
                Some("text/ullink"),
                Some("text/key-value"),
                Some("application/xml"),
                Some("application/octet-stream"),
            ]
        );
        // The body is the line as read, the transport's verb included: it is
        // prose in front of the payload, and the codec reads it there.
        assert_eq!(
            bodies(&batches)[..2],
            [
                b"sending >> 8=FIX.4.2|9=176|35=D|10=203|".to_vec(),
                b"recv ACCOUNT=A1|MSGTYPE=8|SYMBOL=AAPL".to_vec(),
            ]
        );
        assert_eq!(
            bodies(&batches)[4],
            b"no level printed by this plugin".to_vec()
        );
    }

    #[test]
    fn the_currunix_column_prefers_the_header_capture_over_the_handles_own_time() {
        use arrow_array::TimestampNanosecondArray;

        // The expression dates the line, so the instant is the line's own
        // reading resolved into UTC - not the moment the file happened to be
        // written.
        let source = named("dated.log", b"2020-01-02T03:04:05Z id=7 first\n");
        let batch = collect(&source, options(r"^(?<mtime>\S+) id=(?<id>\d+) "))
            .pop()
            .unwrap();
        let schema = batch.schema();
        let names: Vec<&str> = schema
            .fields()
            .iter()
            .map(|field| field.name().as_str())
            .collect();
        // No column of its own: when the record was written is the event's
        // `currunix`, and the `mtime` capture is consumed filling it.
        assert_eq!(names, with_event(&["body", "id"]));
        assert_eq!(
            batch.schema().field(0).data_type(),
            &arrow_schema::DataType::Timestamp(
                arrow_schema::TimeUnit::Nanosecond,
                Some("UTC".into())
            )
        );
        assert_eq!(
            batch
                .column(0)
                .as_any()
                .downcast_ref::<TimestampNanosecondArray>()
                .unwrap()
                .value(0),
            1_577_934_245_000_000_000
        );
    }

    #[test]
    fn an_mtime_capture_that_states_only_a_date_dates_the_line_at_midnight() {
        use arrow_array::TimestampNanosecondArray;

        // A log that dates its lines by the day states no clock, and a date is
        // the instant that day opens: the capture reads midnight in the
        // instant's zone rather than failing the row for want of a clock it
        // never had.
        let source = named("dated.log", b"2020-01-02 id=7 first\n");
        let batch = collect(&source, options(r"^(?<mtime>\S+) id=(?<id>\d+) "))
            .pop()
            .unwrap();
        assert_eq!(
            batch
                .column(0)
                .as_any()
                .downcast_ref::<TimestampNanosecondArray>()
                .unwrap()
                .value(0),
            1_577_923_200_000_000_000
        );

        // The compact spelling a wire writes is the same day and the same
        // reading, where it used to be no reading at all.
        let source = named("dated.log", b"20200102 id=7 first\n");
        let batch = collect(&source, options(r"^(?<mtime>\d+) id=(?<id>\d+) "))
            .pop()
            .unwrap();
        assert_eq!(
            batch
                .column(0)
                .as_any()
                .downcast_ref::<TimestampNanosecondArray>()
                .unwrap()
                .value(0),
            1_577_923_200_000_000_000
        );
    }

    #[test]
    fn a_handle_with_no_modification_time_dates_its_lines_at_the_epoch() {
        use arrow_array::{Array as _, TimestampNanosecondArray};

        // An event happens when it happens, so `currunix` holds no null. A
        // buffer records no modification time and no header dates these lines,
        // and what is left is the instant the count starts from.
        let batch = collect(&named("plain.log", b"first\nsecond\n"), TextOptions::new())
            .pop()
            .unwrap();
        let currunix = batch.column_by_name("currunix").unwrap();
        assert_eq!(currunix.len(), 2);
        assert_eq!(currunix.null_count(), 0);
        assert_eq!(
            currunix
                .as_any()
                .downcast_ref::<TimestampNanosecondArray>()
                .unwrap()
                .values(),
            &[0, 0]
        );
    }

    #[test]
    fn the_currunix_column_falls_back_to_the_handles_own_modification_time() {
        use arrow_array::TimestampNanosecondArray;
        use yggdryl::local::LocalFile;

        let directory = std::env::temp_dir().join("yggdryl_text_mtime");
        std::fs::create_dir_all(&directory).unwrap();
        let path = directory.join("undated.log");
        std::fs::write(&path, b"first\nsecond\n").unwrap();
        let handle = LocalFile::new(&path).unwrap();
        let expected = handle.mtime().expect("a filesystem modification time");

        let batch = collect(&handle, TextOptions::new()).pop().unwrap();
        let values = batch
            .column_by_name("currunix")
            .unwrap()
            .as_any()
            .downcast_ref::<TimestampNanosecondArray>()
            .unwrap();
        // Every row shares the handle's answer: one fact about the object, read
        // once and repeated, never one stat per line.
        assert_eq!(values.values(), &[expected, expected]);

        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn turning_the_mtime_flag_off_frees_the_name_for_an_ordinary_capture() {
        // No read emits a column called `mtime`, flag or no flag: on, the
        // capture of that name dates the line and is spent doing it.
        let batch = collect(&named("plain.log", b"first\n"), TextOptions::new())
            .pop()
            .unwrap();
        assert!(batch.column_by_name("mtime").is_none());

        // Off, nothing claims the name and a capture spelled `mtime` is an
        // ordinary one, typed by its own syntax rather than by the fact it no
        // longer feeds - and it gets the column every unconsumed capture gets.
        let mut captured = options(r"^(?<mtime>\d+) ");
        captured.parse_mtime = false;
        let batch = collect(&named("counted.log", b"77 first\n"), captured)
            .pop()
            .unwrap();
        assert_eq!(batch.schema().field(20).name(), "mtime");
        assert_eq!(
            batch.schema().field(20).data_type(),
            &arrow_schema::DataType::Int64
        );
    }

    #[test]
    fn captures_are_typed_by_name_whatever_fixed_columns_precede_them() {
        // Every fixed column is optional, so a capture's datatype cannot be found
        // by counting the ones in front of it: with the classification columns on,
        // that count was wrong and a capture was parsed at another column's type.
        let mut options = options(r"^(?<seen>\d{4}-\d{2}-\d{2}) id=(?<id>\d+) ");
        options.parse_mimetype = true;
        let batch = collect(&named("wide.log", b"2020-01-02 id=7 first\n"), options)
            .pop()
            .unwrap();
        assert_eq!(
            batch.schema().field_with_name("seen").unwrap().data_type(),
            &arrow_schema::DataType::Date32
        );
        assert_eq!(
            batch.schema().field_with_name("id").unwrap().data_type(),
            &arrow_schema::DataType::Int64
        );
    }

    // --- The one decode path, the plan, and the two new options ---

    mod decoding {

        use arrow_array::{Array as _, StringArray};
        use yggdryl::text::{TextEntries, TextOptions};

        use yggdryl::text::{into_arrow_batch, read_text_lines};

        use super::named;

        fn lines(source: &[u8], options: &TextOptions) -> Vec<yggdryl::text::TextLine> {
            read_text_lines(&named("app.log", source), options)
                .expect("the configuration is settled")
                .map(|line| line.expect("a line decodes"))
                .collect()
        }

        #[test]
        fn every_line_becomes_one_typed_row() {
            let options = TextOptions::new();
            let decoded = lines(b"first\nsecond\nthird\n", &options);
            assert_eq!(decoded.len(), 3);
            assert_eq!(decoded[0].index(), 0);
            assert_eq!(decoded[2].index(), 2);
            assert_eq!(decoded[1].body(), "second");
            // The URL is one shared value, not one rebuilt per line.
            assert_eq!(
                decoded[0].sourceurl().map(ToString::to_string),
                decoded[2].sourceurl().map(ToString::to_string)
            );
        }

        #[test]
        fn a_read_asking_for_no_entry_resolves_the_tree_on_the_first_ask_alone() {
            // Nothing is resolved until asked, and the ask resolves it: the tree
            // the line states, and what the line is classified as. That a read
            // never pays for what it never asks is the allocation suite's pin,
            // `a_line_built_and_read_allocates_nothing_and_its_captures_once`.
            let options = TextOptions::new();
            let decoded = lines(b"35=D|55=AAPL\n", &options);
            assert_eq!(
                decoded[0].entries().map(TextEntries::len),
                Some(2),
                "asked, the tree is read off the body"
            );
            assert_eq!(decoded[0].bodytype().as_str(), "text/fix");
        }

        #[test]
        fn renaming_changes_what_a_column_is_called_and_nothing_else() {
            let options = TextOptions::new()
                .with_renamed_column("body", "payload")
                .with_renamed_column("crosscode", "source");
            let batch = into_arrow_batch(lines(b"hello\n", &options), &options).expect("a batch");
            assert!(batch.column_by_name("payload").is_some());
            assert!(batch.column_by_name("source").is_some());
            assert!(
                batch.column_by_name("body").is_none(),
                "the old name is gone"
            );
            assert_eq!(
                batch
                    .column_by_name("payload")
                    .expect("renamed")
                    .as_any()
                    .downcast_ref::<StringArray>()
                    .expect("still text")
                    .value(0),
                "hello"
            );
        }

        #[test]
        fn a_rename_naming_no_column_is_refused_at_options_time() {
            let options = TextOptions::new().with_renamed_column("nosuch", "x");
            let error = options
                .source_field()
                .expect_err("a rename must name a column");
            let rendered = error.to_string();
            assert!(rendered.contains("nosuch"), "{rendered}");
            assert!(rendered.contains("row-header capture"), "{rendered}");
        }

        #[test]
        fn two_columns_may_not_emit_one_name() {
            let options = TextOptions::new().with_renamed_column("body", "crosscode");
            let error = options.source_field().expect_err("one column per name");
            assert!(error.to_string().contains("twice"), "{error}");
        }

        #[test]
        fn the_classification_column_comes_from_one_scan() {
            let mut options = TextOptions::new();
            options.parse_mimetype = true;
            let decoded = lines(
                b"8=FIX.4.2|35=D|10=001
",
                &options,
            );
            assert_eq!(decoded[0].bodytype().as_str(), "text/fix");
            let batch = into_arrow_batch(decoded, &options).expect("a batch");
            let shape = batch
                .column_by_name("mimetype")
                .expect("the column is there")
                .as_any()
                .downcast_ref::<StringArray>()
                .expect("utf8")
                .value(0)
                .to_owned();
            assert_eq!(shape, "text/fix");
        }

        #[test]
        fn the_schema_a_batch_carries_is_the_schema_the_options_declared() {
            let mut options = TextOptions::new();
            options.start_rownum = Some(1);
            options.parse_mimetype = true;
            let options = options
                .try_with_rowheader(r"^(?<kind>\w+): ")
                .expect("a header");
            let declared = options.source_field().expect("a schema");
            let batch =
                into_arrow_batch(lines(b"trade: 55=AAPL\n", &options), &options).expect("a batch");
            let schema = batch.schema();
            let names: Vec<&str> = schema
                .fields()
                .iter()
                .map(|field| field.name().as_str())
                .collect();
            let declared_fields = declared.fields();
            let expected: Vec<&str> = declared_fields.iter().map(yggdryl::Field::name).collect();
            assert_eq!(
                names, expected,
                "the plan and the schema are one derivation"
            );
        }

        #[test]
        fn an_empty_object_answers_its_columns_and_no_rows() {
            let options = TextOptions::new();
            let batch =
                into_arrow_batch(Vec::<yggdryl::text::TextLine>::new(), &options).expect("a batch");
            assert_eq!(batch.num_rows(), 0);
            assert!(batch.column_by_name("body").is_some());
        }
    }
}
