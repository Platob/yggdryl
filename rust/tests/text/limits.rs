//! `rust/src/text/limits.rs`: the row and byte bounds a text read stops at.

mod text {
    use arrow_array::{Array as _, StringArray, UInt64Array};
    use yggdryl::holder::Buffer;
    use yggdryl::media::IORecordOptions as _;
    use yggdryl::text::TextOptions;

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

    fn seqnums(batches: &[arrow_array::RecordBatch]) -> Vec<Option<u64>> {
        batches
            .iter()
            .flat_map(|batch| {
                let index = batch.schema().index_of("seqnum").unwrap();
                batch
                    .column(index)
                    .as_any()
                    .downcast_ref::<UInt64Array>()
                    .unwrap()
                    .iter()
                    .collect::<Vec<_>>()
            })
            .collect()
    }

    fn dropped(batches: &[arrow_array::RecordBatch]) -> Vec<Option<u64>> {
        batches
            .iter()
            .flat_map(|batch| {
                let index = batch.schema().index_of("dropped_byte_size").unwrap();
                batch
                    .column(index)
                    .as_any()
                    .downcast_ref::<UInt64Array>()
                    .unwrap()
                    .iter()
                    .collect::<Vec<_>>()
            })
            .collect()
    }

    fn strings(batches: &[arrow_array::RecordBatch], name: &str) -> Vec<Option<String>> {
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
                    .map(|value| value.map(str::to_owned))
                    .collect::<Vec<_>>()
            })
            .collect()
    }

    #[test]
    fn a_result_row_limit_does_not_convert_the_following_record() {
        // The second row's sequence number cannot be represented, so converting
        // it would be a refusal; the limit stops the read before that.
        let source = named("limited-values.log", b"A first\nB second\n");
        let mut options = framed(r"^(?<kind>(?-u:.)) ");
        options.start_rownum = Some(i64::MAX);
        options.set_batch_row_size(Some(8));
        options.set_max_row_size(Some(1));

        let batches = collect(&source, options);
        // The body is the line past its header: `A ` matched and came off.
        assert_eq!(bodies(&batches), [b"first".to_vec()]);
        assert_eq!(seqnums(&batches), [Some(u64::try_from(i64::MAX).unwrap())]);
    }

    #[test]
    fn a_physical_row_limit_does_not_convert_the_following_line() {
        let source = named("limited-lines.log", b"A first\nB second\n");
        let mut options = options(r"^(?<kind>(?-u:.)) ");
        options.start_rownum = Some(i64::MAX);
        options.set_batch_row_size(Some(8));
        options.set_max_row_size(Some(1));

        let batches = collect(&source, options);
        assert_eq!(bodies(&batches), [b"first".to_vec()]);
        assert_eq!(seqnums(&batches), [Some(u64::try_from(i64::MAX).unwrap())]);
    }

    #[test]
    fn record_byte_limit_reports_only_bytes_beyond_the_retained_prefix() {
        let source = named("limit.log", b"[A] abc\ndef\n[B] xyz\n");

        // The limit bounds the body, which is what follows the header: the
        // header is retained whole - a record is known by its header, and it
        // comes off the body into its own columns - so a limit of nothing
        // still leaves the record, with an empty body and the whole of it
        // dropped.
        let exact = collect(
            &source,
            framed(r"^\[(?<kind>[A-Z])\] ").with_max_record_byte_size(7),
        );
        assert_eq!(bodies(&exact), [b"abc\ndef".to_vec(), b"xyz".to_vec()]);
        assert_eq!(dropped(&exact), [None, None]);

        let limited = collect(
            &source,
            framed(r"^\[(?<kind>[A-Z])\] ").with_max_record_byte_size(6),
        );
        assert_eq!(bodies(&limited), [b"abc\nde".to_vec(), b"xyz".to_vec()]);
        assert_eq!(dropped(&limited), [Some(1), None]);

        let zero = collect(
            &source,
            framed(r"^\[(?<kind>[A-Z])\] ").with_max_record_byte_size(0),
        );
        assert_eq!(bodies(&zero), [b"".to_vec(), b"".to_vec()]);
        assert_eq!(dropped(&zero), [Some(7), Some(3)]);
    }

    #[test]
    fn oversized_continuations_are_drained_before_the_following_record() {
        let oversized = yggdryl::DEFAULT_STREAM_BATCH_SIZE * 8 + 31;
        let mut bytes = b"[A] begin\n".to_vec();
        bytes.extend(std::iter::repeat_n(b'x', oversized));
        bytes.extend_from_slice(b"\n[B] after\n");
        let source = named("oversized.log", &bytes);
        let options = framed(r"^\[(?<kind>[A-Z])\] ").with_max_record_byte_size(8);

        let batches = collect(&source, options);
        assert_eq!(bodies(&batches), [b"begin\nxx".to_vec(), b"after".to_vec()]);
        assert_eq!(
            dropped(&batches),
            [Some(u64::try_from(oversized - 2).unwrap()), None]
        );
    }

    #[test]
    fn an_oversized_matching_line_retains_only_its_body_prefix() {
        let oversized = yggdryl::DEFAULT_STREAM_BATCH_SIZE * 8 + 31;
        let mut bytes = b"[A] ".to_vec();
        bytes.extend(std::iter::repeat_n(b'x', oversized));
        bytes.extend_from_slice(b"\n[B] after\n");
        let source = named("oversized-header-line.log", &bytes);
        let options = framed(r"^\[(?<kind>[A-Z])\] ").with_max_record_byte_size(8);

        let batches = collect(&source, options);
        assert_eq!(bodies(&batches), [b"xxxxxxxx".to_vec(), b"after".to_vec()]);
        assert_eq!(
            dropped(&batches),
            [Some(u64::try_from(oversized - 8).unwrap()), None]
        );
    }

    #[test]
    fn capped_header_scanning_matches_complete_regex_semantics() {
        let cases: [(&str, &[u8]); 6] = [
            (r"^(?<h>a+)", b"aaaa body\ncontinued"),
            (r"^(?<h>a+?)", b"aaaa body\ncontinued"),
            (r"^(?<h>ab|a)", b"ab body\ncontinued"),
            (r"^(?<h>a+)$", b"aaaa\ncontinued"),
            (r"(?<h>HDR+)", b"prefix HDRRR suffix\ncontinued"),
            (r"^(?<h>a)?START", b"START body\ncontinued"),
        ];
        for (rowheader, source) in cases {
            let source = named("regex-equivalence.log", source);
            let uncapped = collect(&source, framed(rowheader));
            let capped = collect(&source, framed(rowheader).with_max_record_byte_size(1_024));
            assert_eq!(bodies(&capped), bodies(&uncapped), "{rowheader}");
            assert_eq!(
                strings(&capped, "h"),
                strings(&uncapped, "h"),
                "{rowheader}"
            );
            assert!(dropped(&capped).iter().all(Option::is_none), "{rowheader}");
        }

        let header_size = yggdryl::DEFAULT_STREAM_BATCH_SIZE - 1;
        let mut bytes = vec![b'H'; header_size];
        bytes.extend_from_slice(b" body\ncontinued");
        let source = named("window-header.log", &bytes);
        let uncapped = collect(&source, framed(r"^(?<h>H+) "));
        let capped = collect(
            &source,
            framed(r"^(?<h>H+) ").with_max_record_byte_size(1_024),
        );
        assert_eq!(bodies(&capped), bodies(&uncapped));
        assert_eq!(strings(&capped, "h"), strings(&uncapped, "h"));

        let schema = capped[0].schema();
        assert!(schema.field_with_name("h").unwrap().is_nullable());
        assert!(
            schema
                .field_with_name("dropped_byte_size")
                .unwrap()
                .is_nullable()
        );
    }
}
