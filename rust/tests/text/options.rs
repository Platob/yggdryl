//! `rust/src/text/options.rs`: the flat options the `text/plain` encoding
//! takes, and what they refuse.

mod text {
    use arrow_array::{Array as _, StringArray, UInt64Array};
    use yggdryl::IOMedia as _;
    use yggdryl::Timezone;
    use yggdryl::holder::Buffer;
    use yggdryl::media::{IORecordOptions as _, RecordOptions};
    use yggdryl::text::{LeadingFragment, LineSep, TextOptions};

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

    /// The seventeen event columns every line batch opens with, in front of
    /// the line's own: the line is an event of the graph, and a message parsed
    /// out of it contains the same seventeen under the same names and
    /// datatypes.
    const EVENT_COLUMNS: [&str; 16] = [
        "currunix",
        "creaunix",
        "execunix",
        "recdunix",
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
        "srcuuids",
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

    #[test]
    fn options_are_flat_and_validate_rowheader_names() {
        let mut options = TextOptions::new()
            .try_with_rowheader(r"\[(?<level>[A-Z]+)\] (?<id>\d+)")
            .unwrap()
            .try_with_lstrip([r"^\s+"])
            .unwrap()
            .try_with_rstrip([r"\s+$"])
            .unwrap()
            .with_linesep(LineSep::CRLF)
            .with_framing(true)
            .with_leading_fragment(LeadingFragment::Drop)
            .with_max_record_byte_size(1_024)
            .with_autotype(false)
            .with_timezone(Timezone::UTC)
            .with_filter("level = 'WARN'")
            .unwrap();
        options.start_rownum = Some(-3);
        options.set_batch_row_size(Some(7));

        assert_eq!(
            options.rowheader(),
            Some(r"\[(?<level>[A-Z]+)\] (?<id>\d+)")
        );
        assert_eq!(options.lstrip().collect::<Vec<_>>(), [r"^\s+"]);
        assert_eq!(options.rstrip().collect::<Vec<_>>(), [r"\s+$"]);
        assert_eq!(options.linesep(), Some(&LineSep::CRLF));
        assert!(options.framing());
        assert_eq!(options.leading_fragment(), LeadingFragment::Drop);
        assert_eq!(options.max_record_byte_size(), Some(1_024));
        assert!(!options.autotype());
        assert_eq!(options.timezone(), Some(&Timezone::UTC));
        assert_eq!(options.start_rownum, Some(-3));
        assert_eq!(options.batch_row_size(), Some(7));
        // The `where` section is stored flat beside them, and it is part of
        // what one configuration is: two that differ only in their clause are
        // two configurations.
        assert_eq!(options.filter().to_string(), "level = 'WARN'");
        assert_ne!(
            options,
            options.clone().with_filter("level = 'INFO'").unwrap()
        );

        let error = TextOptions::new()
            .try_with_rowheader(r"(?<body>.+)")
            .unwrap_err()
            .to_string();
        assert!(error.contains(
            "distinct from body, dropped_byte_size and the event columns the line derives"
        ));
        // The row's place and the object it came from are stated by the event
        // columns now, so those two names belong to the reader: a capture
        // spelled as one is refused with the rest the line derives, in
        // whatever case it is written.
        for name in ["seqnum", "CROSSCODE"] {
            let error = TextOptions::new()
                .try_with_rowheader(&format!(r"(?<{name}>.+)"))
                .unwrap_err()
                .to_string();
            assert!(error.contains(name), "{name}: {error}");
            assert!(error.contains("event columns the line derives"), "{error}");
        }

        // And the names the row schema no longer spends are the expression's
        // to use: no column of the reader's answers to them any more, so a
        // capture spelled as one is an ordinary capture with a column of its
        // own, named as the caller named it.
        let batches = collect(
            &named("legacy.log", b"file:///elsewhere.log 7 alpha\n"),
            TextOptions::new()
                .try_with_rowheader(r"^(?<sourceurl>\S+) (?<rownum>\d+) ")
                .unwrap(),
        );
        assert_eq!(
            batches[0]
                .schema()
                .fields()
                .iter()
                .map(|field| field.name().as_str())
                .collect::<Vec<_>>(),
            with_event(&["body", "sourceurl", "rownum"])
        );
        assert_eq!(bodies(&batches), [b"alpha".to_vec()]);
        assert_eq!(
            strings(&batches, "sourceurl"),
            [Some("file:///elsewhere.log".into())]
        );
        // Their values stay there, too: a capture wearing the old spelling
        // states its own column and nothing of the event's. The object the
        // line came from is the buffer it was read out of, whatever a capture
        // spells - which is the whole of why the reader stopped spending the
        // name on a column of its own.
        let crosscode = strings(&batches, "crosscode").remove(0).expect("an object");
        assert!(crosscode.starts_with("mem://"), "{crosscode}");
    }

    #[test]
    fn framing_requires_a_rowheader_before_any_source_read() {
        let source = named("unused.log", b"body\n");
        let options: RecordOptions = TextOptions::new().with_framing(true).into();
        let error = source
            .read_arrow_reader(&options)
            .err()
            .unwrap()
            .to_string();
        assert!(error.contains("framing requires a rowheader"));
    }

    #[test]
    fn a_strip_sequence_takes_one_layer_off_at_a_time() {
        // A capture line routinely carries several layers of prose in front of
        // its payload, and one expression matching all of them at once is the
        // expression nobody can read. Each pattern strips from the edge the one
        // before it left.
        let cases: &[(&[&str], &[u8], &[u8])] = &[
            // An arrow alone.
            (&[r"^-->\s*"], b"--> 8=FIX.4.4|35=D|", b"8=FIX.4.4|35=D|"),
            // A stage label, then an arrow.
            (
                &[r"^After \w+\s*", r"^-->\s*"],
                b"After Enrichment --> ACCOUNT=A1|SIDE=1",
                b"ACCOUNT=A1|SIDE=1",
            ),
            // The same two in the other spelling: a label ending in a colon.
            (
                &[r"^After [\w ]+:\s*", r"^-->\s*"],
                b"After the bridge: --> ACCOUNT=A1",
                b"ACCOUNT=A1",
            ),
            // A timestamp, a level, a plugin name, then the arrow.
            (
                &[
                    r"^\d{4}-\d{2}-\d{2} ",
                    r"^[A-Z]+ ",
                    r"^\[\w+\]\s*",
                    r"^-->\s*",
                ],
                b"2026-09-04 INFO [XmlApi] --> 8=FIX.4.2|35=8|",
                b"8=FIX.4.2|35=8|",
            ),
            // A pattern that matches nothing leaves the edge where it was, so a
            // sequence written for the general case still reads the specific one.
            (
                &[r"^After \w+\s*", r"^-->\s*"],
                b"--> ACCOUNT=A1",
                b"ACCOUNT=A1",
            ),
            // A pattern that does not match leaves the edge for the next one, so
            // the sequence is tried in order rather than abandoned at the first
            // miss - which is what lets one listing serve every shape a capture
            // mixes. Here the arrow does not lead, so only the label comes off.
            (
                &[r"^-->\s*", r"^After \w+\s*"],
                b"After Enrichment --> ACCOUNT=A1",
                b"--> ACCOUNT=A1",
            ),
        ];

        for (patterns, line, expected) in cases {
            let options = TextOptions::new()
                .try_with_lstrip(patterns.iter().copied())
                .unwrap();
            let source = Buffer::from_bytes([*line, b"\n"].concat());
            assert_eq!(
                bodies(&collect(&source, options)),
                [expected.to_vec()],
                "{patterns:?}"
            );
        }
    }

    #[test]
    fn a_right_edge_sequence_strips_in_order_too() {
        let options = TextOptions::new()
            .try_with_rstrip([r"\s+$", r"<< queued seq=\d+$", r"\s+$"])
            .unwrap();
        let source = Buffer::from_bytes(b"8=FIX.4.2|35=D|10=203| << queued seq=1092  \n".to_vec());
        assert_eq!(
            bodies(&collect(&source, options)),
            [b"8=FIX.4.2|35=D|10=203|".to_vec()]
        );
    }

    #[test]
    fn adjacent_rows_repeating_a_body_are_dropped_only_when_asked() {
        // `A A B A`: the trailing `A` survives, which is what proves this is one
        // previous digest rather than a set of every body seen.
        let source = Buffer::from_bytes(
            b"8=FIX.4.4|35=D|10=1|\n8=FIX.4.4|35=D|10=1|\n8=FIX.4.4|35=8|10=2|\n8=FIX.4.4|35=D|10=1|\n"
                .to_vec(),
        );

        // Off by default: a row in is a row out.
        assert_eq!(bodies(&collect(&source, TextOptions::new())).len(), 4);

        let mut options = TextOptions::new();
        options.dedup_adjacent = true;
        assert_eq!(
            bodies(&collect(&source, options)),
            [
                b"8=FIX.4.4|35=D|10=1|".to_vec(),
                b"8=FIX.4.4|35=8|10=2|".to_vec(),
                b"8=FIX.4.4|35=D|10=1|".to_vec(),
            ]
        );

        // An empty source drops nothing, and a single row is never its own
        // predecessor.
        let single = Buffer::from_bytes(b"8=FIX.4.4|35=D|10=1|\n".to_vec());
        let mut options = TextOptions::new();
        options.dedup_adjacent = true;
        assert_eq!(bodies(&collect(&single, options)).len(), 1);

        // The digest is of the body after stripping, so two rows differing only
        // in prose a strip removes are one row.
        let prefixed = Buffer::from_bytes(
            b"sending >> 8=FIX.4.4|35=D|10=1|\nrecv >> 8=FIX.4.4|35=D|10=1|\n".to_vec(),
        );
        let mut options = TextOptions::new()
            .try_with_lstrip([r"^(?:sending|recv) >>\s*"])
            .unwrap();
        options.dedup_adjacent = true;
        assert_eq!(bodies(&collect(&prefixed, options)).len(), 1);
    }

    /// A line with no body is no line, in either direction.
    ///
    /// The `body` column is not nullable, and a cell holding the empty string
    /// would make that promise hollow: a row stating nothing is a row nobody can
    /// read back as the line it came from. So the reader never cuts one - a
    /// blank line, or one the strips take whole, is a separator between records
    /// and not a record - the line's own doors refuse one, and a write refuses a
    /// row that carries one. The bytes as cut are what is weighed, before the
    /// row header comes off them: a line that is all header is a line all the
    /// same, stating itself in its capture columns with the body left empty.
    mod body {
        use super::{EVENT_COLUMNS, bodies, collect, named, options, strings, uint64s, with_event};
        use yggdryl::IOMedia as _;
        use yggdryl::text::{Text, TextBytes, TextLine, TextOptions};

        fn line(body: &[u8]) -> yggdryl::Result<TextLine> {
            TextLine::from_bytes(
                7,
                TextBytes::from_bytes(body).expect("a page"),
                std::sync::Arc::new(TextOptions::new()),
            )
        }

        #[test]
        fn a_blank_line_is_a_separator_and_never_a_row() {
            let source = named("blanks.log", b"alpha\n\nbeta\n\n\n");
            let mut read = TextOptions::new();
            read.start_rownum = Some(0);
            let batches = collect(&source, read);
            assert_eq!(bodies(&batches), [b"alpha".to_vec(), b"beta".to_vec()]);
            // The numbering `seqnum` states is the physical line's own, so the
            // gap the blank line left is visible rather than closed over:
            // `beta` is the third line and counts as two, from the zero this
            // read starts at. Zero is no sequence at all, so the row that
            // counts as zero states that by leaving its cell null.
            assert_eq!(uint64s(&batches, "seqnum"), [None, Some(2)]);
        }

        #[test]
        fn a_line_the_strips_take_whole_is_no_record_either() {
            let source = named("stripped.log", b"xxx\nkeep\nxxx\n");
            let mut read = TextOptions::new();
            read.set_lstrip(["^x+"]).expect("an lstrip");
            read.start_rownum = Some(1);
            let batches = collect(&source, read);
            assert_eq!(bodies(&batches), [b"keep".to_vec()]);
            // `keep` is the second physical line and counts as the second row:
            // the lines the strips took are separators, not rows to renumber.
            assert_eq!(uint64s(&batches, "seqnum"), [Some(2)]);
        }

        #[test]
        fn a_count_and_a_read_drop_the_same_lines() {
            // The counting pass keeps no body at all, so the two can only agree
            // if what makes a line a record is what it cut and never what the
            // retained limit kept.
            let source = named("blanks.log", b"alpha\n\nbeta\n\n\n");
            let counted = Text::new(source.clone()).with_options(TextOptions::new());
            assert_eq!(counted.row_size().unwrap(), 2);
            let batches = collect(&source, TextOptions::new());
            let rows: usize = batches.iter().map(arrow_array::RecordBatch::num_rows).sum();
            assert_eq!(rows, 2);
        }

        #[test]
        fn the_line_doors_refuse_a_body_that_states_nothing() {
            let refusal = line(b"").expect_err("no body, no line");
            assert!(refusal.to_string().contains("$[7].body"), "{refusal}");
            assert!(
                refusal.to_string().contains("got an empty one"),
                "{refusal}"
            );
            let mut held = line(b"alpha").expect("a line");
            let refusal = held
                .set_body(TextBytes::new())
                .expect_err("no body, no line");
            assert!(refusal.to_string().contains("$[7].body"), "{refusal}");
            // And the line is the line it was: a refused write changes nothing.
            assert_eq!(held.body(), "alpha");
        }

        #[test]
        fn a_retained_limit_that_keeps_no_body_is_refused_before_a_byte_is_read() {
            // With no row header there is nothing a record is known by, so a
            // limit of zero would answer a line with no body on every row.
            let refusal = named("any.log", b"alpha\n")
                .read_arrow_reader(&TextOptions::new().with_max_record_byte_size(0).into())
                .map(drop)
                .expect_err("a limit that keeps nothing");
            assert!(
                refusal.to_string().contains("max_record_byte_size"),
                "{refusal}"
            );
            // Under a row header the header is always retained, so the same
            // limit keeps the bytes the record is known by and reads. The body
            // is the line past that header, and a limit of zero leaves none of
            // it: each row states its capture over an empty body, which is what
            // a header that consumes the line is for.
            let batches = collect(
                &named("headed.log", b"[A] alpha\n[B] beta\n"),
                options(r"^\[(?<kind>[A-Z])\] ").with_max_record_byte_size(0),
            );
            assert_eq!(bodies(&batches), [b"".to_vec(), b"".to_vec()]);
            assert_eq!(
                strings(&batches, "kind"),
                [Some("A".into()), Some("B".into())]
            );
        }

        #[test]
        fn a_row_that_states_no_body_is_refused_on_the_way_back_out() {
            let source = named("round.log", b"alpha\nbeta\n");
            let batches = collect(&source, TextOptions::new());
            let schema = batches[0].schema();
            let bodies_at = schema.index_of("body").unwrap();
            let mut columns = batches[0].columns().to_vec();
            columns[bodies_at] = std::sync::Arc::new(arrow_array::StringArray::from(vec![
                Some("alpha"),
                Some(""),
            ]));
            let hollow =
                arrow_array::RecordBatch::try_new(std::sync::Arc::clone(&schema), columns).unwrap();
            let mut target = named("written.log", b"");
            let refusal = target
                .write_arrow_reader(
                    yggdryl::arrow::batch_reader(schema, vec![hollow]),
                    yggdryl::IOMode::Overwrite,
                    &TextOptions::new().into(),
                )
                .map(drop)
                .expect_err("a row stating no line");
            assert!(refusal.to_string().contains("$[1].body"), "{refusal}");
        }

        #[test]
        fn every_emitted_column_states_its_nullability_and_says_what_it_holds() {
            let source = named("columns.log", b"2026-01-02T03:04:05Z INFO k=v hello\n");
            let mut read =
                options(r"^(?<mtime>\S+) (?<level>\w+) ").with_max_record_byte_size(1_024);
            read.start_rownum = Some(1);
            read.parse_mimetype = true;
            let batches = collect(&source, read);
            let schema = batches[0].schema();
            assert_eq!(
                schema
                    .fields()
                    .iter()
                    .map(|field| field.name().as_str())
                    .collect::<Vec<_>>(),
                with_event(&["mimetype", "body", "dropped_byte_size", "level"])
            );
            // The five facts every event settles are the five a line always
            // states; everything a line may leave unsaid is nullable, and the
            // two the reader itself answers - what the line was classified as,
            // and the line - are not.
            let required = [
                "currunix",
                "curruuid",
                "crossuuid",
                "currhashcode",
                "crosshashcode",
            ];
            for field in schema.fields() {
                let expected = !(required.contains(&field.name().as_str())
                    || matches!(field.name().as_str(), "mimetype" | "body"));
                assert_eq!(
                    field.is_nullable(),
                    expected,
                    "{} nullability",
                    field.name()
                );
                // Every column says what it holds, the ones a caller named
                // included, so a catalog reading this schema needs nothing else.
                assert!(
                    field.metadata().contains_key("description"),
                    "{} has no description",
                    field.name()
                );
            }
            // And the seventeen a line opens with carry the spelling their own
            // column states, so a line's row and a message's row name one fact
            // one way.
            for name in EVENT_COLUMNS {
                let display = schema.field_with_name(name).unwrap().metadata()["display"].clone();
                assert!(display.eq_ignore_ascii_case(name), "{name} shows {display}");
            }
            // A column that cannot be null never is, whatever the line left
            // unsaid: this row states no level and no `k`.
            let bare = collect(&named("bare.log", b"unmatched\n"), {
                let mut read = options(r"^(?<mtime>\S+) (?<level>\w+) ");
                read.parse_mimetype = true;
                read.start_rownum = Some(1);
                read
            });
            for field in bare[0].schema().fields() {
                if !field.is_nullable() {
                    assert_eq!(
                        bare[0].column_by_name(field.name()).unwrap().null_count(),
                        0,
                        "{} holds a null",
                        field.name()
                    );
                }
            }
        }
    }
}
