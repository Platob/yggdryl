//! The text-line handler, over every terminator form, coding, and container.

use crate::io::{Buffer, IOBase};
use crate::text::{LineSep, Strip, TextLineOptions};
use crate::{Codec, Url};

/// A buffer whose media type carries the codings its name declares.
fn named(name: &str, bytes: &[u8]) -> Buffer {
    let mut handle = Buffer::new().with_media_type(
        Url::from_str(&format!("file:///{name}"))
            .unwrap()
            .media_type(),
    );
    handle.write_all_bytes(bytes).unwrap();
    handle
}

/// Every record of a handle, as owned text.
///
/// The views borrow the reader's window, so a test that wants them all at once
/// copies deliberately - which is the one thing the handler asks a caller to be
/// explicit about.
fn collect(handle: &impl IOBase) -> Vec<String> {
    let mut lines = handle.read_lines().unwrap();
    let mut found = Vec::new();
    while let Some(line) = lines.next() {
        found.push(line.unwrap().text().unwrap().to_owned());
    }
    found
}

/// Drain a reader into owned text.
fn drain<R: std::io::Read>(mut lines: crate::text::TextLines<R>) -> Vec<String> {
    let mut found = Vec::new();
    while let Some(line) = lines.next() {
        found.push(line.unwrap().text().unwrap().to_owned());
    }
    found
}

mod read_lines {
    //! Lines stream off any handle, decoded, one line in memory at a time.

    use super::*;

    #[test]
    fn plain_lines_split_on_newline_and_keep_the_unterminated_tail() {
        let handle = named("trades.jsonl", b"{\"id\":1}\n{\"id\":2}\r\n{\"id\":3}");
        assert_eq!(collect(&handle), ["{\"id\":1}", "{\"id\":2}", "{\"id\":3}"]);
    }

    #[test]
    fn an_absent_resource_yields_no_lines_and_an_empty_line_is_a_line() {
        assert_eq!(collect(&named("void.txt", b"")).len(), 0);
        // A terminator with nothing before it is an empty line, not nothing.
        assert_eq!(collect(&named("gap.txt", b"a\n\nb\n")), ["a", "", "b"]);
    }

    #[test]
    fn a_gzip_named_resource_streams_its_decoded_lines() {
        let encoded = crate::gzip::dump(b"alpha\nbeta\ngamma\n").unwrap();
        let handle = named("words.txt.gz", &encoded);
        assert_eq!(collect(&handle), ["alpha", "beta", "gamma"]);
    }

    #[test]
    fn stacked_codings_peel_outermost_first() {
        // Applied gzip-then-zstd, exactly what the name spells.
        let once = crate::gzip::dump(b"first\nsecond\n").unwrap();
        let twice = crate::zstd::dump(&once).unwrap();
        let handle = named("stack.txt.gz.zst", &twice);
        assert_eq!(collect(&handle), ["first", "second"]);
    }

    #[test]
    fn a_coded_wrapper_streams_without_materializing_and_agrees_with_the_default() {
        let encoded = crate::gzip::dump(b"one\ntwo\n").unwrap();
        let mut inner = Buffer::new();
        inner.write_all_bytes(&encoded).unwrap();
        let coded = crate::io::Coded::new(inner, Codec::Gzip);
        let lines: Vec<String> = drain(coded.read_lines().unwrap());
        assert_eq!(lines, ["one", "two"]);
    }

    #[test]
    fn the_owned_variant_outlives_the_scope_that_built_the_handle() {
        let lines = {
            let handle = named("scoped.txt", b"kept\nalive\n");
            handle.into_read_lines().unwrap()
        };
        assert_eq!(drain(lines), ["kept", "alive"]);
    }

    #[test]
    fn invalid_utf8_is_reported_where_text_is_asked_for_not_where_bytes_are() {
        let handle = named("bad.txt", b"fine\n\xFF\xFE\n");
        let mut lines = handle.read_lines().unwrap();
        assert_eq!(lines.next().unwrap().unwrap().text().unwrap(), "fine");

        // Reading is byte-first: the record arrives, and only asking for its
        // *text* validates. A byte-oriented consumer never pays for UTF-8 it
        // does not need, and never fails on bytes it can handle.
        let line = lines.next().unwrap().unwrap();
        assert_eq!(line.bytes(), b"\xFF\xFE");
        let refused = line.text().unwrap_err().to_string();
        assert!(refused.contains("UTF-8"), "{refused}");
        assert!(refused.contains("row 2"), "{refused}");

        // The stream continues: one unreadable record does not end the read.
        assert!(lines.next().is_none());
    }
}

mod read_lines_matching {
    //! A pattern groups lines into the records it opens.

    use super::*;

    const LOG: &[u8] = b"preamble carried from rotation\n2024-02-01 10:00:00.000_000 [ee] [alpha] first entry\n    at frame one\n    at frame two\n2024-02-01 10:00:01.000_000 [ww] [beta] second entry\n2024-02-01 10:00:02.000_000 [ii] [gamma] third\ntrailing continuation\n";

    const PATTERN: &str = r"^\d{4}-\d{2}-\d{2} \d{2}:\d{2}:\d{2}";

    fn handle() -> Buffer {
        let mut handle =
            Buffer::new().with_media_type(Url::from_str("file:///app.log").unwrap().media_type());
        handle.write_all_bytes(LOG).unwrap();
        handle
    }

    #[test]
    fn a_timestamp_pattern_yields_whole_entries() {
        let records: Vec<String> = drain(handle().read_lines_matching(PATTERN).unwrap());
        assert_eq!(records.len(), 4);
        assert_eq!(records[0], "preamble carried from rotation");
        assert_eq!(
            records[1],
            "2024-02-01 10:00:00.000_000 [ee] [alpha] first entry\n    at frame one\n    at frame two"
        );
        assert_eq!(
            records[3],
            "2024-02-01 10:00:02.000_000 [ii] [gamma] third\ntrailing continuation"
        );
    }

    #[test]
    fn the_owned_variant_and_a_coded_handle_agree() {
        let encoded = crate::gzip::dump(LOG).unwrap();
        let mut coded = Buffer::new()
            .with_media_type(Url::from_str("file:///app.log.gz").unwrap().media_type());
        coded.write_all_bytes(&encoded).unwrap();
        let records: Vec<String> = drain(coded.into_read_lines_matching(PATTERN).unwrap());
        assert_eq!(records.len(), 4);
        assert!(records[1].contains("at frame two"));
    }

    #[test]
    fn an_invalid_pattern_is_an_error_up_front() {
        assert!(handle().read_lines_matching("([").is_err());
    }
}

#[cfg(feature = "arrow")]
mod arrow_lines {
    //! The Arrow projection of matched line records: a text-line surface,
    //! never a fourth record method.

    use super::*;

    use crate::{Scheme, Value};
    use arrow_array::{
        Date32Array, Int32Array, Int64Array, RecordBatch, StringArray, Time64MicrosecondArray,
    };

    const LOG: &[u8] = b"preamble carried from rotation\n\
        2024-02-01 10:00:00.000_000 [ee] [alpha] first entry\n    at frame one\n    at frame two\n\
        2024-02-01 10:00:01.500 [ww] [beta] second entry\n\
        2024-02-01 10:00:02 [ii] [gamma] third\n";

    const PATTERN: &str =
        r"^\d{4}-\d{2}-\d{2} \d{2}:\d{2}:\d{2}\S* \[(?<level>[^\]]+)\] \[(?<logger>[^\]]+)\]";

    /// 2024-02-01T10:00:00, as naive nanoseconds since the Unix epoch.
    const FIRST_UNIX: i64 = 1_706_781_600_000_000_000;
    /// 2024-02-01, as days since the Unix epoch.
    const FIRST_DATE: i32 = 19_754;

    fn options() -> TextLineOptions {
        TextLineOptions::with_pattern(PATTERN).unwrap()
    }

    fn batches(reader: crate::arrow::BatchReader) -> Vec<RecordBatch> {
        reader.collect::<std::result::Result<Vec<_>, _>>().unwrap()
    }

    fn strings(batch: &RecordBatch, index: usize) -> Vec<Option<String>> {
        batch
            .column(index)
            .as_any()
            .downcast_ref::<StringArray>()
            .unwrap()
            .iter()
            .map(|value| value.map(str::to_owned))
            .collect()
    }

    fn int64(batch: &RecordBatch, index: usize) -> Vec<Option<i64>> {
        batch
            .column(index)
            .as_any()
            .downcast_ref::<Int64Array>()
            .unwrap()
            .iter()
            .collect()
    }

    /// Every column except `url`, which is the one column two locations of
    /// the same content legitimately disagree on.
    fn located_columns(batch: &RecordBatch) -> Vec<arrow_array::ArrayRef> {
        batch.columns()[1..].to_vec()
    }

    #[test]
    fn the_projection_parses_headers_captures_and_the_preamble() {
        let batch = &batches(named("app.log", LOG).read_arrow_lines(&options()).unwrap())[0];
        assert_eq!(batch.num_rows(), 4);
        let schema = batch.schema();
        let names: Vec<&str> = schema
            .fields()
            .iter()
            .map(|field| field.name().as_str())
            .collect();
        assert_eq!(
            names,
            [
                "url", "rownum", "date", "time", "unix", "hash", "header", "message", "offset",
                "lines", "level", "logger"
            ]
        );

        // The preamble record: no header, no timestamp, the whole record as
        // the message; url, rownum, hash, message, offset, lines stay filled.
        assert_eq!(int64(batch, 1), [Some(1), Some(2), Some(3), Some(4)]);
        let messages = strings(batch, 7);
        assert_eq!(
            messages[0].as_deref(),
            Some("preamble carried from rotation")
        );
        assert_eq!(
            messages[1].as_deref(),
            Some("first entry\n    at frame one\n    at frame two")
        );
        assert_eq!(messages[3].as_deref(), Some("third"));
        let headers = strings(batch, 6);
        assert_eq!(headers[0], None);
        assert_eq!(
            headers[1].as_deref(),
            Some("2024-02-01 10:00:00.000_000 [ee] [alpha]")
        );

        // The timestamp columns: `_`-grouped microseconds, a millisecond
        // fraction, and bare seconds all read; the preamble stays null.
        assert_eq!(
            int64(batch, 4),
            [
                None,
                Some(FIRST_UNIX),
                Some(FIRST_UNIX + 1_500_000_000),
                Some(FIRST_UNIX + 2_000_000_000),
            ]
        );
        let dates: Vec<Option<i32>> = batch
            .column(2)
            .as_any()
            .downcast_ref::<Date32Array>()
            .unwrap()
            .iter()
            .collect();
        assert_eq!(
            dates,
            [None, Some(FIRST_DATE), Some(FIRST_DATE), Some(FIRST_DATE)]
        );
        let times: Vec<Option<i64>> = batch
            .column(3)
            .as_any()
            .downcast_ref::<Time64MicrosecondArray>()
            .unwrap()
            .iter()
            .collect();
        assert_eq!(
            times,
            [
                None,
                Some(36_000_000_000),
                Some(36_001_500_000),
                Some(36_002_000_000),
            ]
        );

        // Offsets are decoded-stream positions of each record's first line.
        let text = std::str::from_utf8(LOG).unwrap();
        assert_eq!(
            int64(batch, 8),
            [
                Some(0),
                Some(text.find("2024-02-01 10:00:00").unwrap() as i64),
                Some(text.find("2024-02-01 10:00:01").unwrap() as i64),
                Some(text.find("2024-02-01 10:00:02").unwrap() as i64),
            ]
        );
        let lines: Vec<Option<i32>> = batch
            .column(9)
            .as_any()
            .downcast_ref::<Int32Array>()
            .unwrap()
            .iter()
            .collect();
        assert_eq!(lines, [Some(1), Some(3), Some(1), Some(1)]);

        // Captures land as nullable utf8 columns in group order.
        assert_eq!(
            strings(batch, 10),
            [
                None,
                Some("ee".into()),
                Some("ww".into()),
                Some("ii".into())
            ]
        );
        assert_eq!(
            strings(batch, 11),
            [
                None,
                Some("alpha".into()),
                Some("beta".into()),
                Some("gamma".into())
            ]
        );
    }

    #[test]
    fn hash_is_the_stable_fnv_of_the_message_alone() {
        let batch = &batches(named("app.log", LOG).read_arrow_lines(&options()).unwrap())[0];
        let hashes = int64(batch, 5);
        let messages = strings(batch, 7);
        for (hash, message) in hashes.iter().zip(&messages) {
            let expected =
                crate::text::stable_hash_bytes(message.as_deref().unwrap().as_bytes()) as i64;
            assert_eq!(*hash, Some(expected));
        }
        // Equal messages under different headers hash equal, which is what
        // makes the column a dedupe and join key across files and runs.
        let twice = named(
            "twin.log",
            b"2024-02-01 10:00:00 [ee] [a] boom\n2024-02-02 11:30:00 [ww] [b] boom\n",
        );
        let twin = &batches(twice.read_arrow_lines(&options()).unwrap())[0];
        let hashes = int64(twin, 5);
        assert_eq!(hashes[0], hashes[1]);
    }

    #[test]
    fn a_buffer_a_file_and_the_owned_variant_agree() {
        let root = {
            let mut path = std::env::temp_dir();
            path.push(format!("yggdryl-arrow-lines-file-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&path);
            path
        };
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("app.log"), LOG).unwrap();

        let from_buffer = batches(named("app.log", LOG).read_arrow_lines(&options()).unwrap());
        let file = crate::local::File::new(root.join("app.log")).unwrap();
        let borrowed = batches(file.read_arrow_lines(&options()).unwrap());
        let owned = batches(file.into_arrow_lines(&options()).unwrap());

        // The url column names each handle's own location; every other
        // column is identical whichever handle and variant produced it.
        assert_eq!(borrowed, owned);
        assert_eq!(borrowed.len(), 1);
        assert_eq!(
            located_columns(&from_buffer[0]),
            located_columns(&borrowed[0])
        );
        let urls = strings(&borrowed[0], 0);
        assert!(urls[0].as_deref().unwrap().ends_with("app.log"), "{urls:?}");

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn compressed_reads_match_the_uncompressed_batches() {
        let plain = batches(named("app.log", LOG).read_arrow_lines(&options()).unwrap());
        for (name, encoded) in [
            ("app.log.gz", crate::gzip::dump(LOG).unwrap()),
            ("app.log.zst", crate::zstd::dump(LOG).unwrap()),
        ] {
            let coded = batches(named(name, &encoded).read_arrow_lines(&options()).unwrap());
            assert_eq!(coded.len(), plain.len(), "{name}");
            // Byte-identical apart from the url that names the coded leaf -
            // offsets included, because they count *decoded* bytes.
            assert_eq!(
                located_columns(&coded[0]),
                located_columns(&plain[0]),
                "{name}"
            );
        }
    }

    #[test]
    fn crlf_terminators_count_into_offsets_and_out_of_lines() {
        let handle = named(
            "dos.log",
            b"pre\r\n2024-02-01 10:00:00 [ee] [a] one\r\n2024-02-01 10:00:01 [ww] [b] two\r\n",
        );
        let batch = &batches(handle.read_arrow_lines(&options()).unwrap())[0];
        assert_eq!(
            strings(batch, 7),
            [Some("pre".into()), Some("one".into()), Some("two".into())]
        );
        // "pre\r\n" is five decoded bytes, so the second record starts at 5.
        assert_eq!(int64(batch, 8)[1], Some(5));
    }

    #[test]
    fn records_split_into_batches_at_the_declared_size() {
        let handle = named("app.log", LOG);
        let split = batches(
            handle
                .read_arrow_lines(&options().with_batch_size(3))
                .unwrap(),
        );
        // Four records into batches of three: a full batch and the remainder.
        assert_eq!(
            split.iter().map(RecordBatch::num_rows).collect::<Vec<_>>(),
            [3, 1]
        );
        assert_eq!(
            int64(&split[1], 1),
            [Some(4)],
            "rownum continues across batches"
        );
    }

    #[test]
    fn absence_reads_as_zero_batches_with_the_schema_still_answered() {
        let missing = {
            let mut path = std::env::temp_dir();
            path.push(format!(
                "yggdryl-arrow-lines-missing-{}",
                std::process::id()
            ));
            let _ = std::fs::remove_dir_all(&path);
            path
        };
        let options = options();
        let expected = crate::arrow::schema_from_field(options.schema()).unwrap();

        for (name, reader) in [
            (
                "an empty in-memory value",
                named("void.log", b"").read_arrow_lines(&options).unwrap(),
            ),
            (
                "a missing file",
                crate::local::File::new(missing.join("absent.log"))
                    .unwrap()
                    .read_arrow_lines(&options)
                    .unwrap(),
            ),
            (
                "a missing folder",
                crate::local::Folder::new(missing.join("absent"))
                    .unwrap()
                    .read_arrow_lines(&options)
                    .unwrap(),
            ),
        ] {
            assert_eq!(reader.schema(), expected, "{name}");
            assert_eq!(reader.count(), 0, "{name}");
        }
    }

    #[test]
    fn a_malformed_timestamp_in_a_matched_header_is_a_typed_error() {
        let handle = named("bad.log", b"2024-02-30 10:00:00 [ee] [a] boom\n");
        let mut reader = handle.read_arrow_lines(&options()).unwrap();
        let message = reader.next().unwrap().unwrap_err().to_string();
        assert!(message.contains("row 1"), "{message}");
        assert!(message.contains("at byte 8"), "{message}");
        assert!(message.contains("no such day in this month"), "{message}");
        assert!(reader.next().is_none(), "an error ends the stream");
    }

    #[test]
    fn custom_constants_append_typed_columns_to_every_row() {
        let options = options()
            .try_with_custom_fields([
                ("venue", Value::from("XNAS")),
                ("session", Value::from(7_i64)),
            ])
            .unwrap();
        let batch = &batches(named("app.log", LOG).read_arrow_lines(&options).unwrap())[0];
        assert_eq!(batch.num_columns(), 14);
        assert_eq!(
            strings(batch, 12),
            vec![Some("XNAS".to_owned()); 4],
            "the constant lands on every row, matched or preamble"
        );
        assert_eq!(int64(batch, 13), vec![Some(7); 4]);
    }

    #[test]
    fn column_collisions_and_unspellable_customs_are_rejected_up_front() {
        // A capture group shadowing a base column fails at construction.
        let error = TextLineOptions::with_pattern(r"^(?<url>\d+)")
            .unwrap_err()
            .to_string();
        assert!(error.contains("\"url\""), "{error}");
        assert!(error.contains("base column"), "{error}");

        // A custom column shadowing a capture, case-insensitively.
        let error = options()
            .try_with_custom_fields([("LEVEL", Value::from("x"))])
            .unwrap_err()
            .to_string();
        assert!(error.contains("capture group"), "{error}");

        // A datatype the strict Iceberg codec cannot spell is refused with
        // the codec's own words, before any resource is read.
        let error = options()
            .try_with_custom_fields([("count", Value::from(1_u64))])
            .unwrap_err()
            .to_string();
        assert!(error.contains("Iceberg can express"), "{error}");
        assert!(error.contains("uint64"), "{error}");

        // Two captures differing only by ASCII case would be ambiguous in
        // the case-insensitive namespace every cast and selection matches.
        let error = TextLineOptions::with_pattern(r"^(?<level>\d)(?<LEVEL>\d)")
            .unwrap_err()
            .to_string();
        assert!(error.contains("another capture group"), "{error}");

        // A negative decimal scale has no Iceberg spelling; the codec's own
        // rejection lands here, not after the table metadata is committed.
        let error = options()
            .try_with_custom_fields([("px", Value::decimal(5, -2))])
            .unwrap_err()
            .to_string();
        assert!(error.contains("decimal(1, -2)"), "{error}");
        assert!(error.contains("no negative"), "{error}");

        // The v3-only types - a null constant, a nanosecond reading - are
        // refused too: the tables this crate creates are format v2, and a
        // column they cannot legally declare must fail before the first
        // batch.
        for (name, value) in [
            ("note", Value::Null),
            ("stamp", Value::datetime(0, crate::TimeUnit::Nanosecond)),
        ] {
            let error = options()
                .try_with_custom_fields([(name, value)])
                .unwrap_err()
                .to_string();
            assert!(error.contains("format v3"), "{name}: {error}");
        }

        // Failure leaves the options unchanged.
        let mut kept = options();
        assert!(
            kept.set_custom_fields(vec![("hash".into(), Value::from("x"))])
                .is_err()
        );
        assert!(kept.custom_fields().is_empty());
        assert_eq!(kept.schema().field_len(), 12);
    }

    #[test]
    fn a_byte_order_mark_does_not_demote_the_first_entry() {
        let mut content = Vec::from("\u{feff}".as_bytes());
        content.extend_from_slice(b"2024-02-01 10:00:00 [ee] [a] first\n");
        let batch = &batches(
            named("bom.log", &content)
                .read_arrow_lines(&options())
                .unwrap(),
        )[0];
        // The mark is an encoding signature, not a preamble: the anchored
        // pattern still opens the first record.
        assert_eq!(
            strings(batch, 6),
            [Some("2024-02-01 10:00:00 [ee] [a]".into())]
        );
        assert_eq!(strings(batch, 7), [Some("first".into())]);
    }

    #[test]
    fn a_coded_view_projects_the_decoded_value_it_presents() {
        let root = {
            let mut path = std::env::temp_dir();
            path.push(format!("yggdryl-arrow-lines-coded-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&path);
            path
        };
        std::fs::create_dir_all(&root).unwrap();
        let content = b"2024-02-01 10:00:00 [ee] [a] one\n2024-02-01 10:00:01 [ww] [b] two\n";
        std::fs::write(root.join("app.log.gz"), crate::gzip::dump(content).unwrap()).unwrap();

        // The view presents decoded bytes while its location holds the
        // encoded form; the borrowed projection must read what the view
        // presents, exactly as the owned one does.
        let options = options();
        let gzip =
            crate::gzip::Gzip::new(crate::local::File::new(root.join("app.log.gz")).unwrap());
        let borrowed = batches(gzip.read_arrow_lines(&options).unwrap());
        let owned = batches(gzip.into_arrow_lines(&options).unwrap());
        assert_eq!(borrowed, owned);
        assert_eq!(borrowed[0].num_rows(), 2);
        assert_eq!(
            strings(&borrowed[0], 7),
            [Some("one".into()), Some("two".into())]
        );

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn numeric_captures_infer_typed_columns_from_their_sub_patterns() {
        // `[thread_id]` and `(log_level)` fields spelled by the pattern:
        // the whole-body spellings `\d+` and `\d+\.\d+` type themselves,
        // everything else stays text - a closed table, not a guess.
        let options = TextLineOptions::with_pattern(
            r"^\d{4}-\d{2}-\d{2} \d{2}:\d{2}:\d{2} \[(?<thread_id>\d+)\] \((?<log_level>\w+)\) qty=(?<qty>\d+\.\d+)",
        )
        .unwrap();
        let schema = options.schema();
        assert_eq!(
            schema.get_field_by_name("thread_id").unwrap().data_type(),
            &crate::DataType::Int64
        );
        assert_eq!(
            schema.get_field_by_name("log_level").unwrap().data_type(),
            &crate::DataType::Utf8
        );
        assert_eq!(
            schema.get_field_by_name("qty").unwrap().data_type(),
            &crate::DataType::Float64
        );

        let handle = named(
            "typed.log",
            b"preamble\n2024-02-01 10:00:00 [42] (info) qty=1.50 filled\n",
        );
        let batch = &batches(handle.read_arrow_lines(&options).unwrap())[0];
        assert_eq!(
            int64(batch, 10),
            [None, Some(42)],
            "the preamble stays null"
        );
        let quantities: Vec<Option<f64>> = batch
            .column(12)
            .as_any()
            .downcast_ref::<arrow_array::Float64Array>()
            .unwrap()
            .iter()
            .collect();
        assert_eq!(quantities, [None, Some(1.5)]);
        assert_eq!(
            strings(batch, 11),
            [None, Some("info".into())],
            "a \\w+ capture is text"
        );
    }

    #[test]
    fn declared_capture_types_override_in_both_directions() {
        // A declaration types what inference cannot, and turns an inferred
        // numeric back into text.
        let options = TextLineOptions::with_pattern(
            r"^\d{4}-\d{2}-\d{2} \d{2}:\d{2}:\d{2} (?<price>[0-9.]+) ref=(?<reference>\d+)",
        )
        .unwrap()
        .try_with_capture_types([
            ("price", crate::DataType::decimal(9, 2).unwrap()),
            ("reference", crate::DataType::Utf8),
        ])
        .unwrap();
        let schema = options.schema();
        assert_eq!(
            schema.get_field_by_name("price").unwrap().data_type(),
            &crate::DataType::decimal(9, 2).unwrap()
        );
        assert_eq!(
            schema.get_field_by_name("reference").unwrap().data_type(),
            &crate::DataType::Utf8
        );

        let handle = named("prices.log", b"2024-02-01 10:00:00 187.23 ref=007\n");
        let batch = &batches(handle.read_arrow_lines(&options).unwrap())[0];
        let prices = batch
            .column(10)
            .as_any()
            .downcast_ref::<arrow_array::Decimal128Array>()
            .unwrap();
        assert_eq!(prices.value(0), 18_723, "187.23 at scale 2");
        assert_eq!(
            strings(batch, 11),
            [Some("007".into())],
            "the declared text keeps its leading zeroes"
        );
    }

    #[test]
    fn a_capture_the_declared_type_cannot_read_is_an_error_not_a_null() {
        let options = TextLineOptions::with_pattern(
            r"^\d{4}-\d{2}-\d{2} \d{2}:\d{2}:\d{2} \[(?<thread_id>\w+)\]",
        )
        .unwrap()
        .try_with_capture_types([("thread_id", crate::DataType::Int64)])
        .unwrap();
        let handle = named("bad.log", b"2024-02-01 10:00:00 [main]\n");
        let mut reader = handle.read_arrow_lines(&options).unwrap();
        let message = reader.next().unwrap().unwrap_err().to_string();
        assert!(message.contains("main"), "{message}");
        assert!(reader.next().is_none(), "an error ends the stream");
    }

    #[test]
    fn capture_type_declarations_are_validated_like_every_other_column() {
        let options = || TextLineOptions::with_pattern(r"^(?<level>\w+)").unwrap();

        // A name the pattern does not capture.
        let error = options()
            .try_with_capture_types([("missing", crate::DataType::Int64)])
            .unwrap_err()
            .to_string();
        assert!(error.contains("named capture group"), "{error}");

        // A datatype the strict Iceberg codec cannot spell.
        let error = options()
            .try_with_capture_types([("level", crate::DataType::UInt64)])
            .unwrap_err()
            .to_string();
        assert!(error.contains("Iceberg can express"), "{error}");

        // One declaration per capture.
        let error = options()
            .try_with_capture_types([
                ("level", crate::DataType::Int64),
                ("level", crate::DataType::Utf8),
            ])
            .unwrap_err()
            .to_string();
        assert!(error.contains("declared twice"), "{error}");

        // Failure leaves the options unchanged.
        let mut kept = options();
        assert!(
            kept.set_capture_types(vec![("level".into(), crate::DataType::UInt64)])
                .is_err()
        );
        assert!(kept.capture_types().is_empty());
        assert_eq!(
            kept.schema()
                .get_field_by_name("level")
                .unwrap()
                .data_type(),
            &crate::DataType::Utf8
        );
    }

    #[test]
    fn the_schema_builder_answers_without_a_reader_and_the_reader_emits_it() {
        // The standalone builder is the reader's own schema, so a table
        // created from one is exactly what the parse appends.
        let pattern =
            r"^\d{4}-\d{2}-\d{2} \d{2}:\d{2}:\d{2} \[(?<thread_id>\d+)\] (?<log_level>\w+)";
        let standalone = crate::text::schema_from_pattern(pattern).unwrap();
        let options = TextLineOptions::with_pattern(pattern).unwrap();
        assert_eq!(&standalone, options.schema());
        assert_eq!(standalone, options.clone().into_schema());

        let handle = named("built.log", b"2024-02-01 10:00:00 [7] info ok\n");
        let reader = handle.read_arrow_lines(&options).unwrap();
        assert_eq!(
            reader.schema(),
            crate::arrow::schema_from_field(&standalone).unwrap()
        );
        // The typed schema maps to Iceberg unchanged, like every emitted one.
        assert_eq!(
            &standalone.to_scheme_compat(&Scheme::ICEBERG).unwrap(),
            &standalone
        );
    }

    #[test]
    fn the_header_is_matched_within_the_opening_line_alone() {
        // `[^;]+` would happily cross a newline into the continuation line;
        // the header must not, because the grouping opened this record on its
        // first line's own match.
        let options = TextLineOptions::with_pattern(
            r"^\d{4}-\d{2}-\d{2} \d{2}:\d{2}:\d{2} \[(?<tag>[^;]+)\]",
        )
        .unwrap();
        let handle = named("brackets.log", b"2024-02-01 10:00:00 [a] rest\nb] tail\n");
        let batch = &batches(handle.read_arrow_lines(&options).unwrap())[0];
        assert_eq!(strings(batch, 6), [Some("2024-02-01 10:00:00 [a]".into())]);
        assert_eq!(strings(batch, 10), [Some("a".into())]);
        assert_eq!(strings(batch, 7), [Some("rest\nb] tail".into())]);
    }

    #[test]
    fn the_timestamp_capture_override_reads_a_named_group() {
        let options = TextLineOptions::with_pattern(r"^\[(?<level>[^\]]+)\] (?<ts>\S+)")
            .unwrap()
            .try_with_timestamp_capture("ts")
            .unwrap();
        let handle = named("alt.log", b"[ee] 2024-02-01T10:00:00.5 boom\n");
        let batch = &batches(handle.read_arrow_lines(&options).unwrap())[0];
        assert_eq!(int64(batch, 4), [Some(FIRST_UNIX + 500_000_000)]);

        // A name the pattern does not capture is rejected when set.
        let error = TextLineOptions::with_pattern(r"^\[(?<level>[^\]]+)\]")
            .unwrap()
            .try_with_timestamp_capture("stamp")
            .unwrap_err()
            .to_string();
        assert!(error.contains("\"stamp\""), "{error}");
        assert!(error.contains("named capture group"), "{error}");
    }

    #[test]
    fn the_emitted_schema_maps_to_iceberg_unchanged() {
        let options = options()
            .try_with_custom_fields([
                ("venue", Value::from("XNAS")),
                ("session", Value::from(7_i64)),
                ("day", Value::date(19_754)),
            ])
            .unwrap();
        let schema = options.schema();
        // The Iceberg compatibility target maps the whole root without error
        // or change: every column is already a type the format spells.
        assert_eq!(&schema.to_scheme_compat(&Scheme::ICEBERG).unwrap(), schema);

        #[cfg(feature = "iceberg")]
        for field in schema.fields() {
            crate::iceberg::PrimitiveType::from_data_type(field.data_type())
                .unwrap_or_else(|error| panic!("{}: {error}", field.name()));
        }
    }

    mod folders {
        //! A container streams its leaves: name-sorted, lazy, one at a time.

        use super::*;

        fn root(label: &str) -> std::path::PathBuf {
            let mut path = std::env::temp_dir();
            path.push(format!(
                "yggdryl-arrow-lines-{label}-{}",
                std::process::id()
            ));
            let _ = std::fs::remove_dir_all(&path);
            path
        }

        #[test]
        fn a_folder_reads_its_leaves_name_sorted_with_per_leaf_rows() {
            let root = root("sorted");
            std::fs::create_dir_all(&root).unwrap();
            // Written out of name order; the read is name-sorted anyway, and
            // b.log arrives gzip-coded to prove each leaf decodes by its own
            // media type.
            std::fs::write(
                root.join("b.log.gz"),
                crate::gzip::dump(b"2024-02-01 11:00:00 [ii] [b] from b\n").unwrap(),
            )
            .unwrap();
            std::fs::write(
                root.join("a.log"),
                b"2024-02-01 10:00:00 [ee] [a] from a\n2024-02-01 10:00:01 [ww] [a] again\n",
            )
            .unwrap();

            let all = batches(
                crate::local::Folder::new(&root)
                    .unwrap()
                    .read_arrow_lines(&options())
                    .unwrap(),
            );
            // A batch never spans two leaves.
            assert_eq!(
                all.iter().map(RecordBatch::num_rows).collect::<Vec<_>>(),
                [2, 1]
            );
            assert!(
                strings(&all[0], 0)[0]
                    .as_deref()
                    .unwrap()
                    .ends_with("a.log"),
                "name-sorted: a.log first"
            );
            assert!(
                strings(&all[1], 0)[0]
                    .as_deref()
                    .unwrap()
                    .ends_with("b.log.gz")
            );
            // rownum restarts at 1 in each leaf: (url, rownum) is a record
            // identity.
            assert_eq!(int64(&all[0], 1), [Some(1), Some(2)]);
            assert_eq!(int64(&all[1], 1), [Some(1)]);
            assert_eq!(strings(&all[1], 7), [Some("from b".into())]);

            let _ = std::fs::remove_dir_all(&root);
        }

        #[test]
        fn an_empty_folder_reads_as_zero_batches() {
            let root = root("empty");
            std::fs::create_dir_all(&root).unwrap();
            let reader = crate::local::Folder::new(&root)
                .unwrap()
                .read_arrow_lines(&options())
                .unwrap();
            assert_eq!(reader.count(), 0);
            let _ = std::fs::remove_dir_all(&root);
        }

        #[test]
        fn a_later_leaf_is_not_opened_until_the_reader_reaches_it() {
            let root = root("lazy");
            std::fs::create_dir_all(&root).unwrap();
            std::fs::write(root.join("a.log"), b"2024-02-01 10:00:00 [ee] [a] fine\n").unwrap();
            // A leaf whose name declares gzip but whose bytes are not: opening
            // it fails, so its error surfacing *after* a.log's batch proves
            // b was untouched while a streamed.
            std::fs::write(root.join("b.log.gz"), b"not gzip at all").unwrap();

            let mut reader = crate::local::Folder::new(&root)
                .unwrap()
                .read_arrow_lines(&options())
                .unwrap();
            let first = reader.next().unwrap().unwrap();
            assert_eq!(first.num_rows(), 1);
            assert!(
                strings(&first, 0)[0].as_deref().unwrap().ends_with("a.log"),
                "the healthy leaf arrives complete before the broken one is opened"
            );
            assert!(reader.next().unwrap().is_err());
            assert!(reader.next().is_none(), "an error ends the stream");

            let _ = std::fs::remove_dir_all(&root);
        }
    }
}

mod linesep {
    //! The terminator: flexible when unset, exact when pinned, offsets exact
    //! for every form including a resource that mixes them.

    use super::*;

    /// Every record of `bytes` under `linesep`, with its offset.
    fn split(bytes: &[u8], linesep: Option<LineSep>) -> Vec<(u64, String)> {
        let mut options = TextLineOptions::new();
        options.set_linesep(linesep);
        let handle = named("mixed.txt", bytes).into_text_with(options);
        let mut lines = handle.read_lines().unwrap();
        let mut found = Vec::new();
        while let Some(line) = lines.next() {
            let line = line.unwrap();
            found.push((line.offset(), line.text().unwrap().to_owned()));
        }
        found
    }

    #[test]
    fn the_flexible_default_accepts_lf_crlf_and_a_lone_cr() {
        for bytes in [
            b"a\nb\nc".as_slice(),
            b"a\r\nb\r\nc".as_slice(),
            b"a\rb\rc".as_slice(),
        ] {
            let found: Vec<String> = split(bytes, None)
                .into_iter()
                .map(|(_, text)| text)
                .collect();
            assert_eq!(found, ["a", "b", "c"], "{bytes:?}");
        }
    }

    #[test]
    fn all_three_terminators_mix_within_one_resource() {
        // A log rotated on Windows and concatenated on Linux: real corpora are
        // mixed, and the terminator is decided per record rather than sniffed
        // once from the first line.
        let found = split(b"lf\ncrlf\r\ncr\rlast", None);
        assert_eq!(
            found
                .iter()
                .map(|(_, text)| text.as_str())
                .collect::<Vec<_>>(),
            ["lf", "crlf", "cr", "last"]
        );
        // Offsets count the terminator's real width: a `\r\n` counts two.
        assert_eq!(
            found.iter().map(|(offset, _)| *offset).collect::<Vec<_>>(),
            [0, 3, 9, 12]
        );
    }

    #[test]
    fn a_pinned_terminator_is_the_only_one_recognized() {
        // A lone `\n` inside a `\r\n`-pinned resource is content, not a break.
        let found = split(b"one\ntwo\r\nthree", Some(LineSep::CRLF));
        assert_eq!(
            found
                .iter()
                .map(|(_, text)| text.as_str())
                .collect::<Vec<_>>(),
            ["one\ntwo", "three"]
        );
        assert_eq!(
            found.iter().map(|(offset, _)| *offset).collect::<Vec<_>>(),
            [0, 9]
        );
    }

    #[test]
    fn every_pinned_form_splits_including_multi_byte_ones() {
        for (linesep, bytes) in [
            (LineSep::LF, b"a\nb\nc".as_slice()),
            (LineSep::CRLF, b"a\r\nb\r\nc".as_slice()),
            (LineSep::CR, b"a\rb\rc".as_slice()),
            // NUL-delimited, as `find -print0` writes.
            (LineSep::NUL, b"a\0b\0c".as_slice()),
            // ASCII record separator.
            (LineSep::RS, b"a\x1eb\x1ec".as_slice()),
        ] {
            let found: Vec<String> = split(bytes, Some(linesep.clone()))
                .into_iter()
                .map(|(_, text)| text)
                .collect();
            assert_eq!(found, ["a", "b", "c"], "{linesep}");
        }

        // Any non-empty byte string, however long.
        let multi = LineSep::new("<END>").unwrap();
        let found: Vec<String> = split(b"a<END>b<END>c", Some(multi))
            .into_iter()
            .map(|(_, text)| text)
            .collect();
        assert_eq!(found, ["a", "b", "c"]);
    }

    #[test]
    fn an_empty_terminator_is_refused() {
        let refused = LineSep::new("").unwrap_err().to_string();
        assert!(refused.contains("non-empty"), "{refused}");
    }

    #[test]
    fn a_terminator_reads_from_its_escaped_text() {
        use std::str::FromStr;

        assert_eq!(LineSep::from_str(r"\n").unwrap(), LineSep::LF);
        assert_eq!(LineSep::from_str(r"\r\n").unwrap(), LineSep::CRLF);
        assert_eq!(LineSep::from_str(r"\0").unwrap(), LineSep::NUL);
        assert_eq!(LineSep::from_str(r"\x1e").unwrap(), LineSep::RS);
        assert_eq!(LineSep::from_str("<END>").unwrap().as_bytes(), b"<END>");
        assert!(LineSep::from_str(r"\q").is_err());
    }

    #[test]
    fn a_bom_is_stripped_from_the_first_record_and_offsets_keep_counting_it() {
        let found = split("\u{feff}first\nsecond\n".as_bytes(), None);
        assert_eq!(
            found
                .iter()
                .map(|(_, text)| text.as_str())
                .collect::<Vec<_>>(),
            ["first", "second"]
        );
        // The signature is three bytes and every offset counts them, so a seek
        // back to a record's offset still lands on it.
        assert_eq!(
            found.iter().map(|(offset, _)| *offset).collect::<Vec<_>>(),
            [0, 9]
        );
    }

    #[test]
    fn a_final_record_needs_no_terminator_and_an_empty_resource_has_none() {
        assert_eq!(
            split(b"only", None)
                .into_iter()
                .map(|(_, text)| text)
                .collect::<Vec<_>>(),
            ["only"]
        );
        assert!(split(b"", None).is_empty());
        // A terminator with nothing before it is an empty record, not nothing.
        assert_eq!(
            split(b"a\n\nb", None)
                .into_iter()
                .map(|(_, text)| text)
                .collect::<Vec<_>>(),
            ["a", "", "b"]
        );
    }

    #[test]
    fn a_record_larger_than_the_window_still_reads_whole() {
        // The window is 64 KiB; this record is not, so the read grows it -
        // the one case that copies, and it must not lose or split anything.
        let long = "x".repeat(200_000);
        let bytes = format!("short\n{long}\ntail\n");
        let found: Vec<String> = split(bytes.as_bytes(), None)
            .into_iter()
            .map(|(_, text)| text)
            .collect();
        assert_eq!(found.len(), 3);
        assert_eq!(found[1].len(), 200_000);
        assert_eq!(found[2], "tail");
    }

    #[test]
    fn a_write_is_deterministic_and_a_round_trip_holds_for_every_form() {
        let records = ["alpha", "beta", "gamma"];
        for linesep in [
            None,
            Some(LineSep::LF),
            Some(LineSep::CRLF),
            Some(LineSep::CR),
            Some(LineSep::NUL),
            Some(LineSep::new("<END>").unwrap()),
        ] {
            let mut options = TextLineOptions::new();
            options.set_linesep(linesep.clone());
            let mut handle = Buffer::new().into_text_with(options);
            handle.write_lines(records).unwrap();

            // Unset writes the platform-neutral `\n`, never the host's ending:
            // a resource's bytes must not depend on which machine wrote them.
            let expected = linesep
                .as_ref()
                .map_or(b"\n".as_slice(), |sep| sep.as_bytes());
            let mut wanted = Vec::new();
            for record in records {
                wanted.extend_from_slice(record.as_bytes());
                wanted.extend_from_slice(expected);
            }
            assert_eq!(handle.read_all().unwrap(), wanted, "{linesep:?}");

            // Round trip: what was written with L reads back as the same
            // records - and anything written under the default reads back
            // under the flexible default too.
            let mut back = handle.read_lines().unwrap();
            let mut found = Vec::new();
            while let Some(line) = back.next() {
                found.push(line.unwrap().text().unwrap().to_owned());
            }
            assert_eq!(found, records, "{linesep:?}");
        }
    }

    #[test]
    fn a_write_streams_and_an_append_continues_it() {
        let mut handle = Buffer::new().into_text();
        // An iterator, never a `Vec`: a million-line write must not need a
        // million-line collection first.
        handle
            .write_lines((0..1_000).map(|index| format!("row-{index}")))
            .unwrap();
        handle.append_lines(["tail"]).unwrap();

        let mut lines = handle.read_lines().unwrap();
        let mut count = 0;
        let mut last = String::new();
        while let Some(line) = lines.next() {
            last = line.unwrap().text().unwrap().to_owned();
            count += 1;
        }
        assert_eq!(count, 1_001);
        assert_eq!(last, "tail");
    }

    #[test]
    fn a_write_takes_whatever_the_caller_already_holds() {
        // `&str`, `String`, `&[u8]`, and `Vec<u8>` all pass with no conversion
        // at the call site: that is the type inference the method exists for.
        let mut handle = Buffer::new().into_text();
        handle.write_lines(["borrowed"]).unwrap();
        handle.append_lines([String::from("owned")]).unwrap();
        handle.append_lines([b"bytes".as_slice()]).unwrap();
        handle.append_lines([b"vector".to_vec()]).unwrap();
        assert_eq!(
            handle.read_all().unwrap(),
            b"borrowed\nowned\nbytes\nvector\n"
        );
    }
}

mod extraction {
    //! Header, message, strip, hash, and the accessors the columns consume.

    use super::*;

    const PATTERN: &str = r"^\[(?<level>[A-Z]+)\]";

    #[test]
    fn the_message_is_the_record_with_the_header_removed_then_stripped() {
        let options = TextLineOptions::with_pattern(PATTERN).unwrap();
        let handle = named("app.log", b"[WARN] disk almost full  \n").into_text_with(options);
        let mut lines = handle.read_lines().unwrap();
        let line = lines.next().unwrap().unwrap();

        assert_eq!(line.header().unwrap(), Some("[WARN]"));
        assert_eq!(line.message().unwrap(), "disk almost full");
        assert_eq!(line.capture("level").unwrap(), Some("WARN"));
        assert_eq!(line.rownum(), 1);
        assert_eq!(line.line_count(), 1);
    }

    #[test]
    fn each_strip_mode_narrows_only_the_side_it_names() {
        let record = b"[WARN] \t padded \t \n";
        for (lstrip, rstrip, expected) in [
            (Strip::Whitespace, Strip::Whitespace, "padded"),
            (Strip::None, Strip::None, " \t padded \t "),
            (Strip::None, Strip::Whitespace, " \t padded"),
            (Strip::Whitespace, Strip::None, "padded \t "),
            (Strip::Ascii, Strip::Ascii, "padded"),
            (
                Strip::Characters(" ".into()),
                Strip::Characters(" ".into()),
                "\t padded \t",
            ),
        ] {
            let options = TextLineOptions::with_pattern(PATTERN)
                .unwrap()
                .with_lstrip(lstrip.clone())
                .with_rstrip(rstrip.clone());
            let handle = named("app.log", record).into_text_with(options);
            let mut lines = handle.read_lines().unwrap();
            let line = lines.next().unwrap().unwrap();
            assert_eq!(line.message().unwrap(), expected, "{lstrip:?}/{rstrip:?}");
        }
    }

    #[test]
    fn a_strip_change_moves_the_hash_because_the_hash_covers_the_message() {
        let record = b"[WARN]  padded  \n";
        let hash_under = |lstrip: Strip, rstrip: Strip| {
            let options = TextLineOptions::with_pattern(PATTERN)
                .unwrap()
                .with_lstrip(lstrip)
                .with_rstrip(rstrip);
            let handle = named("app.log", record).into_text_with(options);
            let mut lines = handle.read_lines().unwrap();
            lines.next().unwrap().unwrap().hash().unwrap()
        };
        // Deterministic - but *given the options*. Two readers configured
        // differently hash the same log line differently, which is exactly why
        // this is pinned rather than left to be discovered.
        assert_ne!(
            hash_under(Strip::Whitespace, Strip::Whitespace),
            hash_under(Strip::None, Strip::None)
        );
        assert_eq!(
            hash_under(Strip::Whitespace, Strip::Whitespace),
            hash_under(Strip::Whitespace, Strip::Whitespace)
        );
    }

    #[test]
    fn a_header_at_offset_zero_and_one_mid_line_hash_identically() {
        // A two-span message and the equivalent contiguous string must hash the
        // same: a hash that depended on where the header sat in the line would
        // be a silent correctness bug.
        let leading = TextLineOptions::with_pattern(r"^\[(?<level>[A-Z]+)\] ").unwrap();
        let handle = named("a.log", b"[WARN] disk full\n").into_text_with(leading);
        let mut lines = handle.read_lines().unwrap();
        let first = lines.next().unwrap().unwrap();
        assert_eq!(first.message().unwrap(), "disk full");
        let leading_hash = first.hash().unwrap();

        // The same resulting message, with the header spliced out of the
        // middle: two spans rather than one.
        let mid = TextLineOptions::with_pattern(r"\[(?<level>[A-Z]+)\] ").unwrap();
        let handle = named("b.log", b"disk [WARN] full\n").into_text_with(mid);
        let mut lines = handle.read_lines().unwrap();
        let second = lines.next().unwrap().unwrap();
        assert_eq!(second.message().unwrap(), "disk full");
        assert_eq!(second.hash().unwrap(), leading_hash);

        // And the parts are exactly the two spans, never a joined string.
        assert_eq!(second.message_parts().unwrap(), ["disk ", "full"]);
    }

    #[test]
    fn a_separate_header_expression_splits_the_two_roles() {
        // The opening pattern is a cheap anchored check; the header expression
        // is the richer one, run only on the lines that opened a record.
        let options = TextLineOptions::with_pattern(r"^\d{4}-")
            .unwrap()
            .try_with_header(r"^\S+ \[(?<level>[A-Z]+)\] \[(?<logger>\w+)\]")
            .unwrap();
        // The capture columns are the union, opening groups first.
        assert_eq!(
            options.capture_names().collect::<Vec<_>>(),
            ["level", "logger"]
        );

        let handle = named(
            "app.log",
            b"2024-02-01T10:00:00 [WARN] [engine] slow\n2024-02-01T10:00:01 unstructured line\n",
        )
        .into_text_with(options);
        let mut lines = handle.read_lines().unwrap();

        let first = lines.next().unwrap().unwrap();
        assert_eq!(
            first.header().unwrap(),
            Some("2024-02-01T10:00:00 [WARN] [engine]")
        );
        assert_eq!(first.message().unwrap(), "slow");
        assert_eq!(first.capture("logger").unwrap(), Some("engine"));

        // A record the opening pattern matched but the header expression did
        // not takes the unmatched shape: one rule for "no header here".
        let second = lines.next().unwrap().unwrap();
        assert_eq!(second.header().unwrap(), None);
        assert_eq!(
            second.message().unwrap(),
            "2024-02-01T10:00:01 unstructured line"
        );
        assert_eq!(second.capture("level").unwrap(), None);
    }

    #[test]
    fn a_capture_colliding_across_the_two_expressions_is_refused() {
        let refused = TextLineOptions::with_pattern(r"^(?<level>\w+)")
            .unwrap()
            .try_with_header(r"^(?<LEVEL>\w+)")
            .unwrap_err()
            .to_string();
        assert!(refused.contains("LEVEL"), "{refused}");
        assert!(refused.contains("another capture group"), "{refused}");
    }

    #[test]
    fn bytes_never_validate_and_text_validates_once() {
        let handle = named("bad.log", b"\xFF\xFE tail\n").into_text();
        let mut lines = handle.read_lines().unwrap();
        let line = lines.next().unwrap().unwrap();
        // Always available, never validated.
        assert_eq!(line.bytes(), b"\xFF\xFE tail");
        // Every text-shaped accessor reports the same one failure.
        assert!(line.text().is_err());
        assert!(line.message().is_err());
        assert!(line.hash().is_err());
    }

    #[test]
    fn an_owned_record_outlives_the_window_it_was_read_from() {
        let owned = {
            let handle = named("a.log", b"kept\n").into_text();
            let mut lines = handle.read_lines().unwrap();
            lines.next().unwrap().unwrap().into_owned().unwrap()
        };
        assert_eq!(owned.text(), "kept");
        assert_eq!(owned.rownum(), 1);
    }
}

mod log_mode {
    //! Records open where a timestamp opens, with no expression anywhere.

    use super::*;

    const LOG: &[u8] = b"rotated tail carried over\n\
        2024-02-01 10:00:00 [ERROR] [engine] boom\n\
        \tat Handler.invoke(Handler.java:42)\n\
        2 items processed\n\
        2024-02-01T10:00:01.5 [ii] [42] fine\n";

    fn options() -> TextLineOptions {
        TextLineOptions::for_logs()
    }

    #[test]
    fn a_record_opens_where_a_timestamp_opens() {
        let handle = named("app.log", LOG).into_text_with(options());
        let mut lines = handle.read_lines().unwrap();

        // The preamble a rotated file starts with is a record, not a drop.
        let preamble = lines.next().unwrap().unwrap();
        assert_eq!(preamble.text().unwrap(), "rotated tail carried over");
        assert_eq!(preamble.header().unwrap(), None);

        // A stack trace and a continuation that *starts with a digit* both
        // join the entry above them: only a real timestamp parse opens one.
        let entry = lines.next().unwrap().unwrap();
        assert_eq!(entry.line_count(), 3);
        assert!(entry.text().unwrap().contains("2 items processed"));

        let last = lines.next().unwrap().unwrap();
        assert_eq!(last.line_count(), 1);
        assert!(lines.next().is_none());
    }

    #[test]
    fn the_closed_token_table_splits_level_logger_and_thread() {
        let handle = named("app.log", LOG).into_text_with(options());
        let mut lines = handle.read_lines().unwrap();
        let _ = lines.next();

        let entry = lines.next().unwrap().unwrap();
        assert_eq!(
            entry.tokens().unwrap(),
            [Some("ERROR"), Some("engine"), None]
        );

        let last = lines.next().unwrap().unwrap();
        // A bracketed all-digit token is a thread, whatever its position.
        assert_eq!(last.tokens().unwrap(), [Some("ii"), None, Some("42")]);
    }

    #[test]
    fn an_unrecognized_token_is_left_in_the_header_untouched() {
        let handle =
            named("app.log", b"2024-02-01 10:00:00 {json:true} rest\n").into_text_with(options());
        let mut lines = handle.read_lines().unwrap();
        let line = lines.next().unwrap().unwrap();
        // The table is closed: nothing is guessed at, and the token stays put.
        assert_eq!(line.tokens().unwrap(), [None, None, None]);
    }

    #[test]
    fn the_schema_is_static_and_answered_without_a_resource() {
        let schema = options().schema().clone();
        // The recognized columns are a fixed, always-emitted, nullable set -
        // never discovered from what the first batch happened to contain.
        assert!(schema["level"].is_nullable());
        assert!(schema["logger"].is_nullable());
        assert!(schema["thread"].is_nullable());
        // And the same schema comes back with no resource in sight.
        assert_eq!(TextLineOptions::for_logs().schema(), &schema);
    }

    #[test]
    fn a_file_with_no_timestamps_and_an_empty_one_read_as_they_should() {
        let handle = named("plain.log", b"alpha\nbeta\n").into_text_with(options());
        let mut lines = handle.read_lines().unwrap();
        // Nothing opens a record, so the whole file is one preamble.
        let only = lines.next().unwrap().unwrap();
        assert_eq!(only.text().unwrap(), "alpha\nbeta");
        assert!(lines.next().is_none());

        let empty = named("empty.log", b"").into_text_with(options());
        assert!(empty.read_lines().unwrap().next().is_none());
    }

    #[test]
    fn detection_is_per_line_and_survives_mixed_forms_and_mixed_terminators() {
        // Two ISO spellings and three terminators in one resource.
        let handle = named(
            "mixed.log",
            b"2024-02-01 10:00:00 first\r\n2024-02-01T10:00:01.250_000 second\r2024-02-01t10:00:02 third\n",
        )
        .into_text_with(options());
        let mut lines = handle.read_lines().unwrap();
        let mut found = Vec::new();
        while let Some(line) = lines.next() {
            found.push(line.unwrap().line_count());
        }
        assert_eq!(found, [1, 1, 1]);
    }

    #[test]
    fn a_pattern_overrides_detection_and_clearing_it_restores_plain_lines() {
        let mut options = TextLineOptions::for_logs();
        options.set_pattern(Some(r"^\[")).unwrap();
        assert!(!options.is_log_mode());

        // Clearing a pattern restores the plain line surface, never log mode:
        // log mode is a deliberate choice, not something a caller falls into.
        options.set_pattern(None).unwrap();
        assert!(!options.is_log_mode());
        assert!(options.schema().get_field_by_name("level").is_none());
    }
}

#[cfg(feature = "arrow")]
mod batching {
    //! Both bounds close a batch, and whichever trips first wins.

    use arrow_array::Array as _;

    use super::*;

    /// The record-opening pattern every corpus here is read under.
    const PATTERN: &str = r"^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}";

    /// `records` timestamped records, each `payload` long.
    fn even(records: usize, payload: usize) -> Buffer {
        let mut text = String::new();
        for index in 0..records {
            text.push_str("2024-02-01T10:00:00 ");
            text.push_str(&"y".repeat(payload));
            text.push('\n');
            let _ = index;
        }
        named("even.log", text.as_bytes())
    }

    /// A corpus whose record sizes swing by two orders of magnitude.
    fn uneven(records: usize) -> Buffer {
        let mut text = String::new();
        for index in 0..records {
            text.push_str("2024-02-01T10:00:00 short\n");
            if index % 10 == 0 {
                text.push_str("2024-02-01T10:00:00 ");
                text.push_str(&"x".repeat(4_000));
                text.push('\n');
            }
        }
        named("uneven.log", text.as_bytes())
    }

    fn row_counts(handle: &Buffer, options: &TextLineOptions) -> Vec<usize> {
        handle
            .read_arrow_lines(options)
            .unwrap()
            .map(|batch| batch.unwrap().num_rows())
            .collect()
    }

    #[test]
    fn the_row_bound_closes_a_batch_at_exactly_that_many_rows() {
        let options = TextLineOptions::with_pattern(PATTERN)
            .unwrap()
            .with_batch_size(7);
        assert_eq!(row_counts(&even(15, 4), &options), [7, 7, 1]);
    }

    #[test]
    fn the_byte_bound_closes_a_batch_at_the_declared_input_bytes() {
        // Each record is 20 header bytes plus 4 payload plus a terminator: 25.
        let handle = even(10, 4);
        let options = TextLineOptions::with_pattern(PATTERN)
            .unwrap()
            .with_byte_size(75);
        // Three records is 75 bytes, which reaches the bound and closes.
        assert_eq!(row_counts(&handle, &options), [3, 3, 3, 1]);
    }

    #[test]
    fn whichever_bound_trips_first_closes_the_batch() {
        let handle = uneven(30);
        let both = TextLineOptions::with_pattern(PATTERN)
            .unwrap()
            .with_batch_size(8)
            .with_byte_size(8_000);
        let counts = row_counts(&handle, &both);
        // No batch exceeds the row bound, and the wide records close batches
        // early on bytes, so some batches are shorter than the row bound.
        assert!(counts.iter().all(|rows| *rows <= 8), "{counts:?}");
        assert!(counts.iter().any(|rows| *rows < 8), "{counts:?}");
        assert_eq!(counts.iter().sum::<usize>(), 33);
    }

    #[test]
    fn byte_sizing_evens_out_what_row_sizing_makes_lopsided() {
        let handle = uneven(200);
        let by_rows = TextLineOptions::with_pattern(PATTERN)
            .unwrap()
            .with_batch_size(16);
        let by_bytes = TextLineOptions::with_pattern(PATTERN)
            .unwrap()
            .with_byte_size(16 * 1024);

        // The measure that matters is the *input bytes* per batch, which is
        // what byte sizing is for: a row bound lets a batch of stack traces be
        // two orders of magnitude larger than a batch of short lines.
        // Only complete batches: the last one is whatever the corpus had left
        // and says nothing about the bound.
        let spread = |options: &TextLineOptions| {
            let mut sizes = Vec::new();
            for batch in handle.read_arrow_lines(options).unwrap() {
                let batch = batch.unwrap();
                let messages = batch
                    .column_by_name("message")
                    .unwrap()
                    .as_any()
                    .downcast_ref::<arrow_array::StringArray>()
                    .unwrap();
                let bytes: usize = (0..messages.len())
                    .map(|row| messages.value(row).len())
                    .sum();
                sizes.push(bytes);
            }
            sizes.pop();
            (
                sizes.iter().copied().min().unwrap_or(0),
                sizes.iter().copied().max().unwrap_or(0),
            )
        };

        let (rows_low, rows_high) = spread(&by_rows);
        let (bytes_low, bytes_high) = spread(&by_bytes);
        // Cross-multiplied rather than divided, so a spread of 1.98 against
        // 1.00 is not rounded into a tie.
        assert!(
            rows_high * bytes_low > bytes_high * rows_low,
            "row sizing {rows_low}..{rows_high} should swing more than byte sizing \
             {bytes_low}..{bytes_high}"
        );
    }

    #[test]
    fn both_unbounded_is_one_batch_per_leaf() {
        let handle = even(100, 4);
        let mut options = TextLineOptions::with_pattern(PATTERN).unwrap();
        // Explicitly beyond the corpus, which is what "unbounded" means here.
        options.set_batch_size(Some(usize::MAX));
        options.set_byte_size(Some(usize::MAX));
        assert_eq!(row_counts(&handle, &options), [100]);
    }

    #[test]
    fn the_defaults_are_byte_driven_with_the_row_bound_as_a_guard() {
        assert_eq!(TextLineOptions::DEFAULT_BYTE_SIZE, 8 * 1024 * 1024);
        assert_eq!(TextLineOptions::DEFAULT_BATCH_SIZE, 65_536);

        // A small corpus is one batch under the defaults, where the old
        // 1024-row default would have split it - a deliberate change.
        let handle = even(5_000, 4);
        let options = TextLineOptions::with_pattern(PATTERN).unwrap();
        assert_eq!(row_counts(&handle, &options), [5_000]);
    }
}

#[cfg(feature = "arrow")]
mod zones {
    //! `unix` is a real instant when a zone is known, and unchanged when not.

    use super::*;

    fn unix_of(record: &str, options: &TextLineOptions) -> i64 {
        let handle = named("app.log", record.as_bytes());
        let batch = handle
            .read_arrow_lines(options)
            .unwrap()
            .next()
            .unwrap()
            .unwrap();
        batch
            .column_by_name("unix")
            .unwrap()
            .as_any()
            .downcast_ref::<arrow_array::Int64Array>()
            .unwrap()
            .value(0)
    }

    fn naive() -> TextLineOptions {
        TextLineOptions::with_pattern(r"^\S+").unwrap()
    }

    #[test]
    fn unset_reproduces_todays_output_exactly() {
        // The civil reading counted from the epoch, with no zone applied.
        assert_eq!(
            unix_of("2024-02-01T00:00:00 x\n", &naive()),
            1_706_745_600_000_000_000
        );
    }

    #[test]
    fn an_offset_in_the_text_is_honored_and_wins_over_the_option() {
        let with_offset = "2024-02-01T00:00:00+02:00 x\n";
        // Two hours earlier than the same civil reading in UTC.
        let expected = 1_706_745_600_000_000_000 - 2 * 3_600 * 1_000_000_000;
        assert_eq!(unix_of(with_offset, &naive()), expected);

        // The option applies only to timestamps that carry none: an explicit
        // offset is data the log author was explicit about.
        let options = naive()
            .try_with_timezone("-08:00".parse().unwrap())
            .unwrap();
        assert_eq!(unix_of(with_offset, &options), expected);
    }

    #[test]
    fn a_naive_reading_becomes_an_instant_under_a_fixed_offset_and_a_named_zone() {
        let fixed = naive()
            .try_with_timezone("+02:00".parse().unwrap())
            .unwrap();
        assert_eq!(
            unix_of("2024-02-01T00:00:00 x\n", &fixed),
            1_706_745_600_000_000_000 - 2 * 3_600 * 1_000_000_000
        );

        // Paris is +01:00 in February, so the same reading lands an hour back.
        let named_zone = naive()
            .try_with_timezone("Europe/Paris".parse().unwrap())
            .unwrap();
        assert_eq!(
            unix_of("2024-02-01T00:00:00 x\n", &named_zone),
            1_706_745_600_000_000_000 - 3_600 * 1_000_000_000
        );
    }

    #[test]
    fn date_and_time_stay_the_civil_reading_while_unix_moves() {
        let options = naive()
            .try_with_timezone("+02:00".parse().unwrap())
            .unwrap();
        let handle = named("app.log", b"2024-02-01T00:30:00 x\n");
        let batch = handle
            .read_arrow_lines(&options)
            .unwrap()
            .next()
            .unwrap()
            .unwrap();
        let date = batch
            .column_by_name("date")
            .unwrap()
            .as_any()
            .downcast_ref::<arrow_array::Date32Array>()
            .unwrap()
            .value(0);
        // The local date, as written - even though `unix` is on the previous
        // UTC day. Both are right, which is exactly why it is documented.
        assert_eq!(date, 19_754);
        assert!(unix_of("2024-02-01T00:30:00 x\n", &options) < 1_706_745_600_000_000_000);
    }

    #[test]
    fn the_ambiguous_hour_takes_the_earliest_offset_and_a_gap_shifts_forward() {
        let paris = naive()
            .try_with_timezone("Europe/Paris".parse().unwrap())
            .unwrap();

        // 2024-10-27 02:30 local occurs twice in Paris. The earliest offset
        // (+02:00, still summer time) is the stated policy.
        let overlap = unix_of("2024-10-27T02:30:00 x\n", &paris);
        let civil = 1_729_996_200_000_000_000_i64; // 2024-10-27T02:30:00Z
        assert_eq!(overlap, civil - 2 * 3_600 * 1_000_000_000);

        // 2024-03-31 02:30 local never occurs in Paris. The stated policy
        // shifts forward onto the first instant that does.
        // Shifted forward onto the first instant that exists, which is what
        // the smaller offset (+01:00, still winter time) yields.
        let gap = unix_of("2024-03-31T02:30:00 x\n", &paris);
        let gap_civil = 1_711_852_200_000_000_000_i64; // 2024-03-31T02:30:00Z
        assert_eq!(gap, gap_civil - 3_600 * 1_000_000_000);
    }

    #[test]
    fn an_unknown_zone_fails_when_the_options_are_built_not_mid_read() {
        // `Timezone::from_str` validates the *spelling*, so an unknown name
        // parses; the registry check belongs here, before a read streams.
        let unknown: crate::Timezone = "Not/AZone".parse().unwrap();
        let refused = naive().try_with_timezone(unknown).unwrap_err().to_string();
        assert!(refused.contains("Not/AZone"), "{refused}");
        assert!(refused.contains("registry knows"), "{refused}");
    }

    #[test]
    fn the_hash_is_untouched_by_the_zone_because_it_covers_the_message() {
        let record = "2024-02-01T00:00:00 payload\n";
        let hash_of = |options: &TextLineOptions| {
            let handle = named("app.log", record.as_bytes());
            let batch = handle
                .read_arrow_lines(options)
                .unwrap()
                .next()
                .unwrap()
                .unwrap();
            batch
                .column_by_name("hash")
                .unwrap()
                .as_any()
                .downcast_ref::<arrow_array::Int64Array>()
                .unwrap()
                .value(0)
        };
        assert_eq!(
            hash_of(&naive()),
            hash_of(
                &naive()
                    .try_with_timezone("Europe/Paris".parse().unwrap())
                    .unwrap()
            )
        );
    }
}

#[cfg(feature = "arrow")]
mod parity {
    //! The accessors and the columns come from one extraction implementation.

    use arrow_array::Array as _;

    use super::*;

    const LOG: &[u8] = b"tail from rotation\n\
        2024-02-01 10:00:00 [ERROR] [engine] boom  \n\
        \tat frame one\n\
        2024-02-01 10:00:01 [ii] [42] fine\n";

    fn column<'batch>(
        batch: &'batch arrow_array::RecordBatch,
        name: &str,
    ) -> &'batch arrow_array::StringArray {
        batch
            .column_by_name(name)
            .unwrap()
            .as_any()
            .downcast_ref::<arrow_array::StringArray>()
            .unwrap()
    }

    #[test]
    fn every_column_matches_the_accessors_row_for_row() {
        for options in [
            TextLineOptions::for_logs(),
            TextLineOptions::with_pattern(
                r"^\d{4}-\d{2}-\d{2} \d{2}:\d{2}:\d{2} \[(?<level>[^\]]+)\]",
            )
            .unwrap(),
        ] {
            let handle = named("app.log", LOG);
            let batch = handle
                .read_arrow_lines(&options)
                .unwrap()
                .next()
                .unwrap()
                .unwrap();

            let handler = named("app.log", LOG).into_text_with(options.clone());
            let mut lines = handler.read_lines().unwrap();
            let mut row = 0;
            while let Some(line) = lines.next() {
                let line = line.unwrap();
                assert_eq!(
                    column(&batch, "message").value(row),
                    line.message().unwrap().as_ref(),
                    "message row {row}"
                );
                let header = column(&batch, "header");
                match line.header().unwrap() {
                    Some(text) => assert_eq!(header.value(row), text, "header row {row}"),
                    None => assert!(header.is_null(row), "header row {row}"),
                }
                assert_eq!(
                    batch
                        .column_by_name("hash")
                        .unwrap()
                        .as_any()
                        .downcast_ref::<arrow_array::Int64Array>()
                        .unwrap()
                        .value(row),
                    line.hash().unwrap(),
                    "hash row {row}"
                );
                assert_eq!(
                    batch
                        .column_by_name("offset")
                        .unwrap()
                        .as_any()
                        .downcast_ref::<arrow_array::Int64Array>()
                        .unwrap()
                        .value(row),
                    line.offset() as i64,
                    "offset row {row}"
                );
                if options.is_log_mode() {
                    let tokens = line.tokens().unwrap();
                    for (name, expected) in ["level", "logger", "thread"].into_iter().zip(tokens) {
                        let held = column(&batch, name);
                        match expected {
                            Some(value) => assert_eq!(held.value(row), value, "{name} row {row}"),
                            None => assert!(held.is_null(row), "{name} row {row}"),
                        }
                    }
                }
                row += 1;
            }
            assert_eq!(row, batch.num_rows());
        }
    }
}

mod configuration {
    //! A reader is fully specifiable from a document: no code anywhere.

    use super::*;
    use crate::generic::Value;

    #[test]
    fn a_yaml_document_defines_a_whole_reader() {
        // Everything the extractor needs, and not a line of code.
        let document = r#"
pattern: '^(?<stamp>\S+) \[(?<level>[A-Z]+)\]'
linesep: '\r\n'
lstrip: none
rstrip: ascii
byte_size: 1048576
batch_size: 4096
timestamp_capture: stamp
timezone: 'Europe/Paris'
capture_types:
  level: utf8
custom_fields:
  source: gateway
"#;
        let value = crate::yaml::from_str(document).unwrap();
        let options = TextLineOptions::from_value(value).unwrap();

        assert_eq!(options.linesep(), Some(&LineSep::CRLF));
        assert!(matches!(options.lstrip(), Strip::None));
        assert!(matches!(options.rstrip(), Strip::Ascii));
        assert_eq!(options.byte_size(), Some(1_048_576));
        assert_eq!(options.batch_size(), Some(4_096));
        assert_eq!(options.timestamp_capture(), Some("stamp"));
        assert_eq!(
            options.timezone().map(crate::Timezone::as_str),
            Some("Europe/Paris")
        );
        assert_eq!(
            options.capture_names().collect::<Vec<_>>(),
            ["stamp", "level"]
        );
        // The schema follows, with no resource in sight.
        assert_eq!(
            options.schema()["source"].data_type(),
            &crate::DataType::Utf8
        );
    }

    #[test]
    fn the_same_document_reads_the_same_way_in_all_three_formats() {
        let options = TextLineOptions::with_pattern(r"^\[(?<level>[A-Z]+)\]")
            .unwrap()
            .with_byte_size(1 << 20)
            .with_lstrip(Strip::None);
        let value = options.to_value();

        for format in [
            crate::text::Format::Json,
            crate::text::Format::Yaml,
            crate::text::Format::Toml,
        ] {
            let bytes = crate::text::to_vec(&value, format).unwrap();
            let read = crate::text::from_slice(&bytes, format).unwrap();
            let restored = TextLineOptions::from_value(read).unwrap();
            assert_eq!(restored.pattern(), options.pattern(), "{format:?}");
            assert_eq!(restored.byte_size(), options.byte_size(), "{format:?}");
            assert_eq!(restored.schema(), options.schema(), "{format:?}");
        }
    }

    #[test]
    fn only_what_is_set_is_emitted_so_a_default_round_trips_clean() {
        let value = TextLineOptions::new().to_value();
        assert_eq!(value.as_mapping().map(<[_]>::len), Some(0));
        let restored = TextLineOptions::from_value(value).unwrap();
        assert_eq!(restored.schema(), TextLineOptions::new().schema());
    }

    #[test]
    fn every_setting_survives_the_round_trip() {
        let options = TextLineOptions::with_pattern(r"^(?<stamp>\S+) (?<qty>\d+)")
            .unwrap()
            .try_with_header(r"^(?<stamp2>\S+)")
            .unwrap()
            .with_linesep(LineSep::new("<END>").unwrap())
            .with_lstrip(Strip::Characters(" \t".into()))
            .with_rstrip(Strip::None)
            .with_byte_size(4_096)
            .with_batch_size(128)
            .try_with_timestamp_capture("stamp")
            .unwrap()
            .try_with_timezone("+05:30".parse().unwrap())
            .unwrap()
            .try_with_capture_types([("qty", crate::DataType::Int64)])
            .unwrap()
            .try_with_custom_fields([("venue", Value::String("XPAR".into()))])
            .unwrap();

        let restored = TextLineOptions::from_value(options.to_value()).unwrap();
        assert_eq!(restored.pattern(), options.pattern());
        assert_eq!(restored.header(), options.header());
        assert_eq!(restored.linesep(), options.linesep());
        assert_eq!(restored.byte_size(), options.byte_size());
        assert_eq!(restored.batch_size(), options.batch_size());
        assert_eq!(restored.timestamp_capture(), options.timestamp_capture());
        assert_eq!(restored.timezone(), options.timezone());
        assert_eq!(restored.capture_types(), options.capture_types());
        assert_eq!(restored.custom_fields(), options.custom_fields());
        assert_eq!(restored.schema(), options.schema());
        // And dumping again is byte-identical.
        assert_eq!(restored.to_value(), options.to_value());
    }

    #[test]
    fn log_mode_round_trips_as_an_explicit_opening() {
        let value = TextLineOptions::for_logs().to_value();
        assert_eq!(
            value.get_key_str("opening").and_then(Value::as_str),
            Some("timestamp")
        );
        assert!(TextLineOptions::from_value(value).unwrap().is_log_mode());
    }

    #[test]
    fn an_unknown_key_and_a_bad_value_are_refused_naming_the_option() {
        let unknown =
            Value::from_mapping([(Value::String("batch-size".into()), Value::U64(10))]).unwrap();
        let refused = TextLineOptions::from_value(unknown)
            .unwrap_err()
            .to_string();
        assert!(refused.contains("batch-size"), "{refused}");
        assert!(refused.contains("a known option"), "{refused}");

        let wrong = Value::from_mapping([(
            Value::String("byte_size".into()),
            Value::String("lots".into()),
        )])
        .unwrap();
        let refused = TextLineOptions::from_value(wrong).unwrap_err().to_string();
        assert!(refused.contains("byte_size"), "{refused}");
        assert!(refused.contains("a count"), "{refused}");

        let bad_capture = Value::from_mapping([
            (
                Value::String("pattern".into()),
                Value::String(r"^\[".into()),
            ),
            (
                Value::String("capture_types".into()),
                Value::from_mapping([(
                    Value::String("absent".into()),
                    Value::String("int64".into()),
                )])
                .unwrap(),
            ),
        ])
        .unwrap();
        let refused = TextLineOptions::from_value(bad_capture)
            .unwrap_err()
            .to_string();
        assert!(refused.contains("absent"), "{refused}");
    }
}
