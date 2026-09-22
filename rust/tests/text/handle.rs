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
    fn a_retained_where_clause_keeps_rows_on_the_read_and_nothing_on_the_decode() {
        // The wrapper holds one configuration and every surface reads it, but
        // they do not read the same part of it. The `where` clause is the
        // read's: the count answers what the object holds, and the one decode
        // answers every line, because neither is the result.
        let text = Text::new(named("app.log", b"alpha\nbravo\ncharlie\n")).with_options(
            TextOptions::new()
                .with_filter("body like 'b%'")
                .unwrap()
                .with_select("body")
                .unwrap()
                .with_max_row_size(1),
        );

        // The count is the whole media, as it is for every encoding: the
        // clause, the projection and the bound are the read's alone.
        assert_eq!(text.row_size().unwrap(), 3);
        assert_eq!(
            text.read_text_lines()
                .unwrap()
                .map(|line| line.unwrap().body().to_owned())
                .collect::<Vec<_>>(),
            ["alpha", "bravo", "charlie"]
        );
        let batches = text
            .read_arrow_reader(&text.record_options().unwrap())
            .unwrap()
            .collect::<std::result::Result<Vec<_>, _>>()
            .unwrap();
        assert_eq!(bodies(&batches), [b"bravo".to_vec()]);
    }

    #[test]
    fn adjacent_deduplication_runs_under_the_where_clause_and_not_over_it() {
        // Dropping a repeated body is the decode's, and keeping a row is the
        // read's, so the clause answers the deduplicated stream rather than the
        // physical one: the second `alpha` is gone before the clause sees it.
        let source = named("app.log", b"alpha\nalpha\nbravo\n");
        let mut deduplicated = TextOptions::new().with_filter("body = 'alpha'").unwrap();
        deduplicated.dedup_adjacent = true;
        let mut repeated = deduplicated.clone();
        repeated.dedup_adjacent = false;

        assert_eq!(bodies(&collect(&source, deduplicated)), [b"alpha".to_vec()]);
        assert_eq!(
            bodies(&collect(&source, repeated)),
            [b"alpha".to_vec(), b"alpha".to_vec()]
        );
    }

    #[test]
    fn a_where_clause_matching_no_line_answers_the_row_schema_and_no_rows() {
        // An empty result is the columns and no rows, never an error and never
        // a missing schema: the shape was settled before a byte was read.
        let source = named("app.log", b"alpha\nbravo\n");
        let options = TextOptions::new().with_filter("body = 'nothing'").unwrap();
        let declared = source.read_arrow_field(&options.clone().into()).unwrap();

        let reader = source.read_arrow_reader(&options.into()).unwrap();
        let schema = arrow_array::RecordBatchReader::schema(&reader);
        assert_eq!(schema.fields().len(), declared.field_len());
        let rows: usize = reader.map(|batch| batch.unwrap().num_rows()).sum();
        assert_eq!(rows, 0);
    }

    #[test]
    fn a_where_clause_reads_the_name_a_column_is_emitted_under() {
        // A rename decides what a column is called, and the clause binds
        // against the schema a read publishes, so the new name is the name the
        // clause has to use - and the old one names no column at all.
        let source = named("app.log", b"alpha\nbravo\n");
        let renamed = TextOptions::new().with_renamed_column("body", "payload");

        let batches = collect(
            &source,
            renamed.clone().with_filter("payload = 'bravo'").unwrap(),
        );
        assert_eq!(strings(&batches, "payload"), [Some("bravo".to_owned())]);
        let error = source
            .read_arrow_reader(&renamed.with_filter("body = 'bravo'").unwrap().into())
            .err()
            .expect("a column no schema declares")
            .to_string();
        assert!(error.contains("body"), "{error}");
    }

    #[test]
    fn a_where_clause_reads_the_typed_columns_a_line_states() {
        // Not the body alone: the object a line came from, the instant it is
        // dated by, and the bytes a bounded record dropped are columns the
        // clause binds against at their own datatypes.
        let mut options = TextOptions::new();
        options.set_max_record_byte_size(Some(3));
        let source = named("app.log", b"alpha\nbravo\n");

        let clause = "sourceurl like 'mem://%' and mtime is null and dropped_byte_size = 2";
        assert_eq!(
            bodies(&collect(&source, options.with_filter(clause).unwrap())),
            [b"alp".to_vec(), b"bra".to_vec()]
        );
    }

    #[test]
    fn a_row_bound_counts_the_rows_a_where_clause_kept() {
        // The bound is the plan's `limit`, and it counts result rows: ten with
        // a clause means the first ten matching lines, not the matches among
        // the first ten lines.
        let source = named("app.log", b"alpha\nbravo\nbronze\nbrass\n");
        let options = TextOptions::new()
            .with_filter("body like 'b%'")
            .unwrap()
            .with_max_row_size(2);

        let batches = collect(&source, options);
        assert_eq!(bodies(&batches), [b"bravo".to_vec(), b"bronze".to_vec()]);
    }

    #[test]
    fn a_where_clause_reads_the_logical_record_framing_built() {
        // Framing decides what a row is before the clause decides whether to
        // keep it: the body the clause reads is the joined record, and the
        // header's capture is the record's own.
        let source = named("app.log", b"[A] first\ncontinued\n[B] second\n");
        let options = framed(r"^\[(?<kind>[A-Z])\] ")
            .with_filter("kind = 'A'")
            .unwrap();

        let batches = collect(&source, options);
        assert_eq!(bodies(&batches), [b"[A] first\ncontinued".to_vec()]);
    }

    #[test]
    fn a_coded_text_read_keeps_the_rows_its_where_clause_names() {
        // A content coding is its own read seam, and it applies the clauses
        // itself rather than handing back everything it decoded.
        let source = named(
            "app.log.gz",
            &Codec::Gzip.dump(b"alpha\nbravo\ncharlie\n").unwrap(),
        );
        let options = TextOptions::new().with_filter("body like 'b%'").unwrap();

        assert_eq!(
            bodies(&collect(&source, options.clone())),
            [b"bravo".to_vec()]
        );
        let coded = yggdryl::coding::Coding::new(
            named(
                "app.log",
                &Codec::Gzip.dump(b"alpha\nbravo\ncharlie\n").unwrap(),
            ),
            Codec::Gzip,
        );
        let batches = coded
            .read_arrow_reader(&options.into())
            .unwrap()
            .collect::<std::result::Result<Vec<_>, _>>()
            .unwrap();
        assert_eq!(bodies(&batches), [b"bravo".to_vec()]);
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
    fn a_folder_of_logs_answers_one_where_clause_across_its_leaves() {
        use yggdryl::local::Folder;

        let mut root = Folder::temporary().unwrap().path().unwrap();
        root.push(format!("yggdryl-clause-folder-{}", std::process::id()));
        let mut folder = Folder::new(&root).unwrap();
        folder.remove(true).unwrap();
        folder
            .child_by_path("a.log")
            .unwrap()
            .write_all_bytes(b"alpha\nbravo\n")
            .unwrap();
        folder
            .child_by_path("b.log")
            .unwrap()
            .write_all_bytes(b"bronze\ncharlie\n")
            .unwrap();

        // One clause over the tree, not one per leaf: the rows come back in
        // listing order with every leaf's non-matching lines already gone.
        let batches = collect(
            &folder,
            TextOptions::new().with_filter("body like 'b%'").unwrap(),
        );
        assert_eq!(bodies(&batches), [b"bravo".to_vec(), b"bronze".to_vec()]);
    }

    #[test]
    fn a_partition_equality_prunes_text_leaves_before_one_is_opened() {
        use yggdryl::local::Folder;

        let mut root = Folder::temporary().unwrap().path().unwrap();
        root.push(format!("yggdryl-clause-hive-{}", std::process::id()));
        let mut folder = Folder::new(&root).unwrap();
        folder.remove(true).unwrap();
        folder
            .child_by_path("year=2024/a.log")
            .unwrap()
            .write_all_bytes(b"first\n")
            .unwrap();
        folder
            .child_by_path("year=2025/b.log")
            .unwrap()
            .write_all_bytes(b"second\n")
            .unwrap();

        // The directory name is a column of the row, and an equality over it
        // is answered about the path: the 2025 leaf is never decoded, and the
        // rest of the clause would still run over the rows.
        let batches = collect(
            &folder,
            TextOptions::new().with_filter("year = '2024'").unwrap(),
        );
        assert_eq!(bodies(&batches), [b"first".to_vec()]);
        assert_eq!(strings(&batches, "year"), [Some("2024".to_owned())]);
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
