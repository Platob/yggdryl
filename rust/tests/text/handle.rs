//! `rust/src/text/handle.rs`: stateful plain-text record media over one byte
//! handle.

mod text {
    use arrow_array::{Array as _, Int64Array, StringArray, UInt64Array};
    use yggdryl::holder::Buffer;
    use yggdryl::media::{IORecordOptions as _, RecordOptions};
    use yggdryl::text::{Text, TextOptions};
    use yggdryl::{Codec, DataType, StructType};
    use yggdryl::{IOBase as _, IOMedia as _};

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

    fn rownums(batches: &[arrow_array::RecordBatch]) -> Vec<i64> {
        batches
            .iter()
            .flat_map(|batch| {
                let index = batch.schema().index_of("rownum").unwrap();
                batch
                    .column(index)
                    .as_any()
                    .downcast_ref::<Int64Array>()
                    .unwrap()
                    .values()
                    .to_vec()
            })
            .collect()
    }

    fn uint64s(batches: &[arrow_array::RecordBatch], name: &str) -> Vec<Option<u64>> {
        batches
            .iter()
            .flat_map(|batch| {
                let index = batch.schema().index_of(name).unwrap();
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

    fn assert_text_buffer(_: &Text<Buffer>) {}

    /// The alignment a binding can actually give the read it holds.
    ///
    /// A binding does not box what it hands back: PyO3 places this struct inside
    /// an object CPython allocated, and CPython's allocator aligns to 16 bytes.
    /// A field asking for more - a vector-width searcher is the one that did -
    /// makes every use of the read a misaligned dereference, which a release
    /// build quietly tolerates on x86 and a debug build aborts the process over.
    /// The rule is cheap to keep and invisible to break, so it is pinned here
    /// rather than discovered in a binding's test suite.
    #[test]
    fn a_read_fits_the_alignment_an_object_allocator_gives() {
        assert!(
            align_of::<yggdryl::text::TextLines>() <= 16,
            "a text read asks to be aligned to {} bytes, and an object allocator gives 16",
            align_of::<yggdryl::text::TextLines>()
        );
    }

    #[test]
    fn repeated_text_conversion_reconfigures_one_wrapper() {
        let text = named("app.log", b"body\n")
            .into_text()
            .into_text()
            .into_text_with(options(r"(?<value>body)"));

        assert_text_buffer(&text);
        assert_eq!(text.options().rowheader(), Some(r"(?<value>body)"));
    }

    #[test]
    fn text_row_size_counts_logical_records_when_framing_is_enabled() {
        let source = named("count.log", b"[A] first\ncontinued\n[B] second\n");
        let text = Text::new(source).with_options(framed(r"^\[(?<kind>[A-Z])\] "));

        assert_eq!(text.row_size().unwrap(), 2);
    }

    #[test]
    fn text_row_size_ignores_row_value_conversion_and_retains_no_bodies() {
        let source = named("count-raw.log", b"A first\ncontinued\n\xFF second\n");
        let mut options = framed(r"^(?<kind>(?-u:.)) ");
        options.start_rownum = Some(i64::MAX);
        let text = Text::new(source).with_options(options);

        // The second output row cannot be represented by the configured rownum,
        // and its capture is a byte the decode would have to repair. Neither
        // changes the number of records, because counting converts nothing.
        assert_eq!(text.row_size().unwrap(), 2);
    }

    #[test]
    fn gzip_and_zstd_framing_decode_the_same_logical_records() {
        let decoded = b"[A] first\ncontinued\n[B] second\n";
        for (name, codec) in [
            ("records.log.gz", Codec::Gzip),
            ("records.log.zst", Codec::Zstd),
        ] {
            let source = named(name, &codec.dump(decoded).unwrap());
            let batches = collect(&source, framed(r"^\[(?<kind>[A-Z])\] "));
            assert_eq!(
                bodies(&batches),
                [b"[A] first\ncontinued".to_vec(), b"[B] second".to_vec()]
            );
        }
    }

    #[test]
    fn folder_leaves_never_share_framing_state_and_restart_physical_rownums() {
        use yggdryl::local::Folder;

        let mut root = Folder::temporary().unwrap().path().unwrap();
        root.push(format!("yggdryl-framed-folder-{}", std::process::id()));
        let mut folder = Folder::new(&root).unwrap();
        folder.remove(true).unwrap();
        let mut first = folder.child_by_path("a.log").unwrap();
        first.write_all_bytes(b"[A] first\ncontinued in a").unwrap();
        let mut second = folder.child_by_path("b.log").unwrap();
        second.write_all_bytes(b"leading in b\n[B] second").unwrap();

        let mut options = framed(r"^\[(?<kind>[A-Z])\] ");
        options.start_rownum = Some(1);
        let batches = collect(&folder, options);
        assert_eq!(
            bodies(&batches),
            [
                b"[A] first\ncontinued in a".to_vec(),
                b"leading in b".to_vec(),
                b"[B] second".to_vec(),
            ]
        );
        assert_eq!(rownums(&batches), [1, 1, 2]);
        assert_eq!(uint64s(&batches, "seqnum"), [Some(1), Some(1), Some(2)]);
        let sourceurls = strings(&batches, "sourceurl");
        assert_eq!(strings(&batches, "crosscode"), sourceurls);
        assert_ne!(sourceurls[0], sourceurls[1]);
        assert_eq!(sourceurls[1], sourceurls[2]);

        folder.remove(true).unwrap();
    }

    #[test]
    fn generic_record_writes_use_only_the_text_body() {
        let mut target = named("out.txt", b"old");
        let mut options: RecordOptions = TextOptions::new().into();
        let field = StructType::from_fields([
            DataType::utf8().required_field("sourceurl"),
            DataType::Int64.required_field("rownum"),
            DataType::utf8().required_field("body"),
        ])
        .map(DataType::from)
        .unwrap()
        .required_field("row");
        options.set_field(field);
        let rows = [
            yggdryl::Scalar::from_struct([
                ("sourceurl", yggdryl::Scalar::from("input")),
                ("rownum", yggdryl::Scalar::from(1_i64)),
                ("body", yggdryl::Scalar::from("first")),
            ])
            .unwrap(),
            yggdryl::Scalar::from_struct([
                ("sourceurl", yggdryl::Scalar::from("input")),
                ("rownum", yggdryl::Scalar::from(2_i64)),
                ("body", yggdryl::Scalar::from("second")),
            ])
            .unwrap(),
        ];
        target.overwrite_records(rows, &options).unwrap();
        assert_eq!(target.read_all_bytes().unwrap(), b"first\nsecond\n");
    }

    #[test]
    fn a_text_read_is_shaped_by_its_select_and_where_sections() {
        // The select casts, aliases and reads the row header's captures; the
        // where clause names an alias, so it runs after the projection.
        let source = named(
            "app.log",
            b"[INFO] id=7 first\n[WARN] id=9 second\n[INFO] id=11 third\nplain\n",
        );
        let mut options = options(r"\[(?<level>[A-Z]+)\] id=(?<id>\d+)");
        options.start_rownum = Some(1);
        let options = options
            .with_select(
                "sourceurl, cast(rownum as int32) as n, trim(body) as line, level, id * 10 as tenfold int64",
            )
            .unwrap()
            .with_filter("n > 1 and line like '%d' and level is not null")
            .unwrap();
        let batches = collect(&source, options);
        let batch = arrow_select::concat::concat_batches(&batches[0].schema(), &batches).unwrap();
        let names: Vec<_> = batch
            .schema()
            .fields()
            .iter()
            .map(|field| field.name().clone())
            .collect();
        assert_eq!(names, ["sourceurl", "n", "line", "level", "tenfold"]);
        assert_eq!(batch.column(1).data_type(), &arrow_schema::DataType::Int32);
        let lines = batch
            .column(2)
            .as_any()
            .downcast_ref::<StringArray>()
            .unwrap()
            .iter()
            .map(|line| line.unwrap().to_owned())
            .collect::<Vec<_>>();
        assert_eq!(lines, ["[WARN] id=9 second", "[INFO] id=11 third"]);
        let tenfold = batch
            .column(4)
            .as_any()
            .downcast_ref::<Int64Array>()
            .unwrap()
            .values()
            .to_vec();
        assert_eq!(tenfold, [90, 110]);
        // A buffer is located in memory, and the column says so.
        let url = batch
            .column(0)
            .as_any()
            .downcast_ref::<StringArray>()
            .unwrap();
        assert!(url.value(0).starts_with("mem://"), "{}", url.value(0));
    }
}
