//! `rust/src/holder/mod.rs`: the concrete `Holder` and what it converts into.

mod coded_holders {

    mod held {
        use yggdryl::holder::buffered::BufferedOptions;
        use yggdryl::holder::{Buffer, Holder};
        use yggdryl::{Codec, IOBase, Level, MimeType, Url};

        const PLAIN: &[u8] = b"[INFO] alpha\n[WARN] beta\n";

        fn named(name: &str, bytes: Vec<u8>) -> Holder {
            Holder::buffer(
                Buffer::from_bytes(bytes).with_media_type(
                    Url::from_str(&format!("file:///{name}"))
                        .unwrap()
                        .media_type(),
                ),
            )
        }

        fn compressed(name: &str, codec: Codec) -> Holder {
            named(name, codec.dump(PLAIN).unwrap())
        }

        #[test]
        fn a_held_coding_comes_from_the_name_and_presents_decoded_bytes() {
            for (name, codec) in [("app.log.gz", Codec::Gzip), ("app.log.zst", Codec::Zstd)] {
                let source = compressed(name, codec);
                assert_eq!(source.read_all_bytes().unwrap(), codec.dump(PLAIN).unwrap());

                let decoded = source.into_coded();
                assert!(matches!(decoded, Holder::Coded(_)), "{name}");
                assert_eq!(decoded.read_all_bytes().unwrap(), PLAIN, "{name}");
                assert_eq!(decoded.size(), PLAIN.len() as u64, "{name}");
                assert_eq!(decoded.media_type().base(), &MimeType::PLAIN_TEXT, "{name}");
                assert!(decoded.media_type().encodings().is_empty(), "{name}");
            }
        }

        #[test]
        fn a_name_declaring_no_coding_passes_its_bytes_through() {
            let decoded = named("app.log", PLAIN.to_vec()).into_coded();
            assert_eq!(decoded.read_all_bytes().unwrap(), PLAIN);
            assert_eq!(decoded.media_type().base(), &MimeType::PLAIN_TEXT);
        }

        #[test]
        fn repeating_the_conversion_never_decodes_twice() {
            let decoded = compressed("app.log.gz", Codec::Gzip)
                .into_coded()
                .into_coded()
                .into_coded_with(Codec::Zstd, Level::BEST);
            assert_eq!(decoded.read_all_bytes().unwrap(), PLAIN);
        }

        #[test]
        fn a_page_cache_stays_outside_the_coding() {
            let decoded = compressed("app.log.gz", Codec::Gzip)
                .buffered(BufferedOptions::default())
                .into_coded();
            match &decoded {
                Holder::Buffered(buffered) => {
                    assert!(matches!(buffered.handle(), Holder::Coded(_)));
                }
                other => panic!("expected a cache outside the coding, got {other:?}"),
            }
            assert_eq!(decoded.read_all_bytes().unwrap(), PLAIN);
        }

        #[test]
        fn a_coded_holder_reads_its_text_records_through_the_decoded_view() {
            use yggdryl::IOMedia as _;

            let decoded = compressed("app.log.gz", Codec::Gzip).into_coded();
            let options = decoded.record_options().unwrap();
            assert!(matches!(options, yggdryl::media::RecordOptions::Text(_)));

            let bodies = decoded
                .read_arrow_reader(&options)
                .unwrap()
                .map(|batch| {
                    let batch = batch.unwrap();
                    let index = batch.schema().index_of("body").unwrap();
                    arrow_array::cast::as_string_array(batch.column(index))
                        .iter()
                        .map(|value| value.unwrap().to_owned())
                        .collect::<Vec<_>>()
                })
                .collect::<Vec<_>>()
                .concat();
            assert_eq!(bodies, ["[INFO] alpha", "[WARN] beta"]);
            assert_eq!(decoded.row_size().unwrap(), 2);
        }
    }
}

mod vocabulary {
    use yggdryl::holder::buffered::BufferedOptions;
    use yggdryl::holder::{Buffer, Holder};
    use yggdryl::{Codec, IOBase, MimeType, Url};

    const PLAIN: &[u8] = b"symbol,price\nAAPL,1\n";

    /// A handle holding `bytes` under the media type `name` declares.
    fn named(name: &str, bytes: Vec<u8>) -> (Holder, yggdryl::MediaType) {
        let url = Url::from_str(&format!("file:///{name}")).unwrap();
        let media_type = url.media_type();
        let holder = Holder::buffer(Buffer::from_bytes(bytes).with_media_type(media_type.clone()));
        (holder, media_type)
    }

    /// The composed handle for `name` over the coding its suffix declares.
    fn composed(name: &str) -> Holder {
        let codec = Codec::from_url(&Url::from_str(&format!("file:///{name}")).unwrap());
        let (holder, _) = named(name, codec.dump(PLAIN).unwrap());
        holder.into_declared_media()
    }

    #[test]
    fn a_coding_and_a_text_base_compose_as_text_over_the_decoded_view() {
        for name in ["trades.txt.gz", "trades.txt.zst", "trades.txt.zz"] {
            let handle = composed(name);
            match &handle {
                Holder::Text(text) => assert!(
                    matches!(text.handle(), Holder::Coded(_)),
                    "{name} held {:?}",
                    text.handle()
                ),
                other => panic!("expected text over a coding for {name}, got {other:?}"),
            }
            assert_eq!(handle.read_all_bytes().unwrap(), PLAIN, "{name}");
            assert_eq!(handle.media_type().base(), &MimeType::PLAIN_TEXT, "{name}");
            assert!(handle.media_type().encodings().is_empty(), "{name}");
        }
    }

    #[test]
    fn a_coding_alone_composes_only_the_decoded_view() {
        let handle = composed("archive.bin.gz");
        assert!(matches!(handle, Holder::Coded(_)), "{handle:?}");
        assert_eq!(handle.read_all_bytes().unwrap(), PLAIN);
    }

    #[test]
    fn a_record_encoding_alone_composes_only_the_media() {
        let (handle, _) = named("rows.arrows", Vec::new());
        assert!(matches!(handle.into_declared_media(), Holder::Media(_)));

        // A name composes to the implementation this build carries. Parquet is an
        // opt-in feature, so without it `rows.parquet` names a record encoding
        // nothing here can read and the handle is left as the bytes it is.
        let (handle, _) = named("rows.parquet", Vec::new());
        let handle = handle.into_declared_media();
        #[cfg(feature = "parquet")]
        assert!(matches!(handle, Holder::Media(_)), "{handle:?}");
        #[cfg(not(feature = "parquet"))]
        assert!(matches!(handle, Holder::Buffer(_)), "{handle:?}");

        let (handle, _) = named("rows.txt", PLAIN.to_vec());
        assert!(matches!(handle.into_declared_media(), Holder::Text(_)));

        let (handle, _) = named("rows.xlsx", Vec::new());
        match handle.into_declared_media() {
            Holder::Media(media) => assert!(
                matches!(
                    media.as_ref(),
                    yggdryl::media::Media::Registered(wrapper)
                        if wrapper.medium().name() == "excel"
                ),
                "{media:?}"
            ),
            other => panic!("expected the workbook medium, got {other:?}"),
        }
    }

    #[test]
    fn a_workbook_name_promotes_to_the_excel_medium_and_answers_records() {
        use yggdryl::media::{IORecordOptions as _, Media};
        use yggdryl::{DataType, IOMedia as _, Scalar, StructType};

        let field = DataType::from(
            StructType::from_fields([
                DataType::Int64.required_field("id"),
                DataType::utf8().nullable_field("symbol"),
            ])
            .unwrap(),
        )
        .required_field("row");

        // Promotion reads nothing and stacks nothing: the workbook medium
        // stands directly over the bytes the name was given.
        let (handle, media_type) = named("trades.xlsx", Vec::new());
        assert_eq!(media_type.base(), &MimeType::XLSX);
        let mut held = handle.into_media().into_media().into_declared_media();
        match &held {
            Holder::Media(media) => match media.as_ref() {
                Media::Registered(wrapper) if wrapper.medium().name() == "excel" => {
                    assert!(matches!(wrapper.handle(), Holder::Buffer(_)));
                }
                other => panic!("expected the workbook medium, got {other:?}"),
            },
            other => panic!("expected a retained media holder, got {other:?}"),
        }
        assert_eq!(held.media_type().base(), &MimeType::XLSX);

        // A handle holding nothing is an empty workbook, not a failure.
        let options = held.record_options().unwrap();
        assert!(options.settings::<yggdryl::excel::ExcelOptions>().is_some());
        assert_eq!(held.row_size().unwrap(), 0);
        assert_eq!(held.read_arrow_reader(&options).unwrap().count(), 0);

        let options = options.with_field(field.clone());
        held.overwrite_records(
            [
                Scalar::from_sequence([Scalar::from(1_i64), Scalar::from("AAPL")]),
                Scalar::from_sequence([Scalar::from(2_i64), Scalar::Null]),
            ],
            &options,
        )
        .unwrap();
        // The bytes are the package: a ZIP local file header first.
        assert_eq!(held.read_range_bytes(0, 4).unwrap(), *b"PK\x03\x04");
        assert_eq!(held.row_size().unwrap(), 2);
        assert_eq!(held.column_size().unwrap(), 2);

        let rows: Vec<String> =
            yggdryl::StreamChunkedSerie::from_serie(held.read_serie(Some(&options)).unwrap())
                .expect("native record stream")
                .into_chunks()
                .flat_map(|serie| {
                    let serie = serie.unwrap();
                    (0..serie.len())
                        .map(|index| serie.scalar(index).unwrap().into_json().unwrap())
                        .collect::<Vec<_>>()
                })
                .collect();
        assert_eq!(rows, ["[1,\"AAPL\"]", "[2,null]"]);

        // Undeclared, the sheet states its own field: the header names the
        // columns, a number cell is float64, and a column a row lacks is
        // nullable.
        let inferred = held
            .read_arrow_field(&held.record_options().unwrap())
            .unwrap();
        assert_eq!(
            inferred,
            DataType::from(
                StructType::from_fields([
                    DataType::Float64.required_field("id"),
                    DataType::utf8().nullable_field("symbol"),
                ])
                .unwrap(),
            )
            .required_field(yggdryl::media::DEFAULT_ROOT_NAME)
        );
    }

    #[test]
    fn a_name_declaring_neither_is_returned_unchanged() {
        let (handle, _) = named("notes.json", PLAIN.to_vec());
        let handle = handle.into_declared_media();
        assert!(matches!(handle, Holder::Buffer(_)), "{handle:?}");
        assert_eq!(handle.read_all_bytes().unwrap(), PLAIN);
    }

    #[test]
    fn repeating_the_composition_stacks_nothing() {
        let codec = Codec::Gzip;
        let (handle, _) = named("trades.txt.gz", codec.dump(PLAIN).unwrap());
        let once = handle.into_declared_media();
        let twice = once.into_declared_media();
        match &twice {
            Holder::Text(text) => match text.handle() {
                Holder::Coded(coded) => assert!(matches!(coded.handle(), Holder::Buffer(_))),
                other => panic!("expected one coding under the text, got {other:?}"),
            },
            other => panic!("expected text over a coding, got {other:?}"),
        }
        assert_eq!(twice.read_all_bytes().unwrap(), PLAIN);
    }

    #[test]
    fn a_page_cache_stays_outside_the_composition() {
        let (handle, _) = named("trades.txt.gz", Codec::Gzip.dump(PLAIN).unwrap());
        let handle = handle
            .buffered(BufferedOptions::default())
            .into_declared_media();
        match &handle {
            Holder::Buffered(buffered) => match buffered.handle() {
                Holder::Text(text) => assert!(matches!(text.handle(), Holder::Coded(_))),
                other => panic!("expected text over a coding under the cache, got {other:?}"),
            },
            other => panic!("expected the cache outermost, got {other:?}"),
        }
        assert_eq!(handle.read_all_bytes().unwrap(), PLAIN);
    }

    #[test]
    fn composing_never_resolves_the_location() {
        // The media type is the caller's, so composing never resolves what is
        // there: an absent location composes, reads empty, and stays lazy.
        let url = Url::from_str("file:///yggdryl-absent-composition/trades.txt.gz").unwrap();
        let handle = Holder::local(url.into_path().unwrap())
            .unwrap()
            .into_declared_media();
        assert!(matches!(handle, Holder::Text(_)), "{handle:?}");
        assert!(handle.read_all_bytes().unwrap().is_empty());
    }

    /// A fresh, empty temporary root of this test's own.
    fn tree(label: &str) -> std::path::PathBuf {
        let mut root = yggdryl::local::LocalFolder::temporary()
            .unwrap()
            .path()
            .unwrap();
        root.push(format!("yggdryl-holder-{label}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        root
    }

    fn remove(root: &std::path::Path) {
        yggdryl::local::LocalFolder::new(root)
            .expect("a local container")
            .remove(true)
            .expect("a removable tree");
    }

    fn held(url: &Url) -> Holder {
        Holder::from_url(url, std::iter::empty::<(&str, &str)>()).unwrap()
    }

    #[test]
    fn a_location_spelling_a_container_composes_its_leaves_media_and_no_coding() {
        // A pattern's suffix names what each leaf holds: the record encoding
        // its leaves are read by, and a coding each leaf takes off itself - so
        // no coding goes over the stream of them.
        for spelled in [
            "file:///yggdryl-absent-composition/logs/*.log.gz",
            "file:///yggdryl-absent-composition/logs/**/*.txt.gz",
        ] {
            match held(&Url::from_str(spelled).unwrap()).into_declared_media() {
                Holder::Text(text) => {
                    assert!(matches!(text.handle(), Holder::LocalPath(_)), "{spelled}");
                }
                other => panic!("{spelled}: expected text over the location, got {other:?}"),
            }
        }

        // A location ending in a slash is a container whatever its last name
        // says, and composes no coding either.
        let slashed = Url::from_str("file:///yggdryl-absent-composition/logs.txt.gz/").unwrap();
        let handle = held(&slashed).into_declared_media();
        assert!(!matches!(handle, Holder::Coded(_)), "{handle:?}");
        if let Holder::Text(text) = &handle {
            assert!(matches!(text.handle(), Holder::LocalPath(_)), "{handle:?}");
        }

        // The same on a filesystem location spelled with its slash. (A
        // filesystem path is opaque, so a `*` in one is a name, not a pattern.)
        use std::sync::Arc;
        use yggdryl::fs::{BoundLocation, FileSystem, MemoryFileSystem};
        let filesystem: Arc<dyn FileSystem> = Arc::new(MemoryFileSystem::new());
        let location = BoundLocation::new(filesystem, "logs.txt.gz/", None::<String>).unwrap();
        let handle = yggdryl::fs::located(location).into_declared_media();
        assert!(!matches!(handle, Holder::Coded(_)), "{handle:?}");

        // A leaf's name still composes as the value it names.
        let url = Url::from_str("file:///yggdryl-absent-composition/trades.txt.gz").unwrap();
        match held(&url).into_declared_media() {
            Holder::Text(text) => assert!(matches!(text.handle(), Holder::Coded(_))),
            other => panic!("expected text over a coding, got {other:?}"),
        }
    }

    #[test]
    fn a_pattern_of_one_encoding_reads_only_the_leaves_of_that_encoding() {
        use std::sync::Arc;

        use arrow_array::{Int64Array, RecordBatch};
        use yggdryl::IOMedia as _;

        let root = tree("encoding-pattern");
        let write = |relative: &str, values: &[i64]| {
            let schema = Arc::new(arrow_schema::Schema::new(vec![arrow_schema::Field::new(
                "id",
                arrow_schema::DataType::Int64,
                false,
            )]));
            let batch =
                RecordBatch::try_new(schema, vec![Arc::new(Int64Array::from(values.to_vec()))])
                    .unwrap();
            let mut leaf = yggdryl::local::LocalPath::new(root.join(relative)).unwrap();
            let options = leaf.record_options().unwrap();
            leaf.overwrite_arrow_reader(
                yggdryl::arrow::batch_reader(batch.schema(), [batch]),
                &options,
            )
            .unwrap();
        };
        write("a.arrows", &[1, 2]);
        write("day=2/b.arrows", &[3]);
        // Private trees are left out of every walk by default, whatever they
        // hold, as a marker and a note beside the data are by their encoding.
        write(".venv/hidden.arrows", &[9]);
        write(".config/hidden.arrows", &[9]);
        std::fs::write(root.join("notes.txt"), b"not a row").unwrap();
        std::fs::write(root.join("_SUCCESS"), b"").unwrap();

        // The pattern names the encoding, so the composed reader is that
        // encoding's, and it reads that encoding's leaves as one table.
        let pattern = Url::from_path(&root)
            .unwrap()
            .joinpath("**/*.arrows")
            .unwrap();
        let handle = held(&pattern).into_declared_media();
        assert!(matches!(handle, Holder::Media(_)), "{handle:?}");
        assert_eq!(handle.row_size().unwrap(), 3);
        let options = handle.record_options().unwrap();
        let read: usize = handle
            .read_arrow_reader(&options)
            .unwrap()
            .map(|batch| batch.unwrap().num_rows())
            .sum();
        assert_eq!(read, 3);

        // The folder, which names no encoding, finds the same one beneath it
        // and reads the same leaves.
        let folder = held(&Url::from_path(&root).unwrap()).into_declared_media();
        assert_eq!(folder.row_size().unwrap(), 3);

        remove(&root);
    }

    #[test]
    fn a_pattern_of_coded_leaves_reads_each_leaf_decoded() {
        let root = tree("coded-pattern");
        std::fs::write(root.join("a.log.gz"), Codec::Gzip.dump(b"a1\n").unwrap()).unwrap();
        std::fs::write(root.join("b.log.gz"), Codec::Gzip.dump(b"b1\n").unwrap()).unwrap();
        std::fs::write(root.join("c.log"), b"c1\n").unwrap();

        // Each matched leaf is decoded on its own - one gzip decoder over the
        // two would end with the first member - and the plain leaf is not
        // matched.
        let pattern = Url::from_path(&root).unwrap().joinpath("*.log.gz").unwrap();
        let handle = held(&pattern).into_declared_media();
        assert!(!matches!(handle, Holder::Coded(_)), "{handle:?}");
        assert_eq!(handle.read_all_bytes().unwrap(), b"a1\nb1\n");

        remove(&root);
    }

    #[test]
    fn a_coding_stated_over_a_location_ending_in_a_slash_leaves_each_leaf_its_own() {
        use yggdryl::IOMedia as _;
        use yggdryl::media::RecordOptions;
        use yggdryl::text::TextOptions;

        let root = tree("coded-slash");
        let logs = root.join("logs");
        std::fs::create_dir_all(&logs).unwrap();
        std::fs::write(logs.join("a.log"), b"a1\na2\n").unwrap();
        std::fs::write(logs.join("b.log.gz"), Codec::Gzip.dump(b"b1\n").unwrap()).unwrap();

        let url = Url::from_str(&format!("{}/", Url::from_path(&logs).unwrap())).unwrap();
        let handle = Holder::from_url(&url, [("codec", "gzip")]).unwrap();
        // The stated coding is still parsed, and goes over no stream of
        // leaves: each leaf takes off the coding its own name declares.
        assert!(matches!(handle, Holder::LocalPath(_)), "{handle:?}");
        assert!(Holder::from_url(&url, [("codec", "rot13")]).is_err());
        assert_eq!(
            handle.read_all_bytes().unwrap(),
            b"a1
a2
b1
"
        );

        let options = RecordOptions::from(TextOptions::default());
        let bodies = handle
            .read_arrow_reader(&options)
            .unwrap()
            .map(|batch| {
                let batch = batch.unwrap();
                let index = batch.schema().index_of("body").unwrap();
                arrow_array::cast::as_string_array(batch.column(index))
                    .iter()
                    .map(|value| value.unwrap().to_owned())
                    .collect::<Vec<_>>()
            })
            .collect::<Vec<_>>()
            .concat();
        assert_eq!(bodies, ["a1", "a2", "b1"]);

        remove(&root);
    }

    #[test]
    fn every_wrapper_keeps_the_filesystem_location_it_stands_on() {
        use std::sync::Arc;

        use yggdryl::fs::{BoundLocation, FileSystem, MemoryFileSystem};

        let filesystem: Arc<dyn FileSystem> = Arc::new(MemoryFileSystem::new());
        let stored = Codec::Gzip.dump(PLAIN).unwrap();
        let mut writer = filesystem
            .open_output_stream("trades.txt.gz", None)
            .unwrap();
        let mut offset = 0;
        while offset < stored.len() {
            offset += writer.write(&stored[offset..]).unwrap();
        }
        writer.close().unwrap();

        let location = BoundLocation::new(filesystem, "trades.txt.gz", None::<String>).unwrap();

        // A wrapper answers where the bytes live, or every filesystem accessor -
        // the raw path, the URI, the info call - goes blank the moment a handle is
        // composed.
        let composed = yggdryl::fs::located(location.clone()).into_declared_media();
        assert!(matches!(composed, Holder::Text(_)), "{composed:?}");
        assert_eq!(
            composed.bound_location().map(BoundLocation::path),
            Some("trades.txt.gz")
        );
        assert_eq!(composed.read_all_bytes().unwrap(), PLAIN);

        let cached = yggdryl::fs::located(location).buffered(BufferedOptions::default());
        assert_eq!(
            cached.bound_location().map(BoundLocation::path),
            Some("trades.txt.gz")
        );
    }

    #[cfg(feature = "parquet")]
    #[test]
    fn a_coding_around_parquet_is_left_for_the_writer_to_refuse() {
        // Parquet compresses internally, so a decoded view over `.parquet.gz`
        // would hide a name no other Parquet reader can open.
        let (handle, _) = named("trades.parquet.gz", Vec::new());
        let handle = handle.into_declared_media();
        assert!(matches!(handle, Holder::Buffer(_)), "{handle:?}");
        assert_eq!(handle.codec(), Codec::Gzip);
    }

    #[test]
    fn a_coding_around_a_workbook_is_left_for_the_excel_doors_to_refuse() {
        // A workbook is a ZIP package deflated inside: the same rule, so the
        // name reaches the doors that refuse it rather than a decoded view.
        let (handle, _) = named("trades.xlsx.gz", Vec::new());
        let handle = handle.into_declared_media();
        assert!(matches!(handle, Holder::Buffer(_)), "{handle:?}");
        assert_eq!(handle.codec(), Codec::Gzip);
    }

    #[test]
    fn a_composed_absent_location_is_still_absent() {
        // Every wrapper reports the storage role of what it stands on, so a
        // composed handle for a location that is not there answers `Unknown` and
        // reads empty rather than claiming to be a file.
        let url = Url::from_str("file:///yggdryl-absent-composition/trades.txt.gz").unwrap();
        let handle = Holder::local(url.into_path().unwrap())
            .unwrap()
            .into_declared_media();
        assert_eq!(handle.kind(), yggdryl::IOKind::Unknown);
        assert!(handle.read_all_bytes().unwrap().is_empty());
    }

    #[test]
    fn a_composed_whole_read_decodes_once() {
        use std::sync::atomic::{AtomicUsize, Ordering};

        /// A handle that counts how many times its bytes are streamed.
        #[derive(Debug)]
        struct Counted {
            handle: Buffer,
            streams: std::sync::Arc<AtomicUsize>,
        }

        impl yggdryl::IOMedia for Counted {
            yggdryl::impl_default_iomedia!();
        }

        impl IOBase for Counted {
            yggdryl::delegate_iobase!(handle: create_bytes, pread, pwrite, size, capacity, reserve, truncate, uri, url,
                media_type, set_media_type, flush, kind);

            fn pstream_bytes(
                &self,
                position: u64,
                batch_size: usize,
            ) -> yggdryl::Result<yggdryl::ByteStream<'_>> {
                self.streams.fetch_add(1, Ordering::Relaxed);
                self.handle.pstream_bytes(position, batch_size)
            }
        }

        let streams = std::sync::Arc::new(AtomicUsize::new(0));
        let url = Url::from_str("file:///trades.txt.gz").unwrap();
        let source = Counted {
            handle: Buffer::from_bytes(Codec::Gzip.dump(PLAIN).unwrap())
                .with_media_type(url.media_type()),
            streams: std::sync::Arc::clone(&streams),
        };

        // Text over a coding answers the whole read through the coding rather than
        // through the trait's `size`-then-read default, which would decode the
        // value once to measure it and again to read it.
        let text = yggdryl::text::Text::new(yggdryl::coding::Coding::new(source, Codec::Gzip));
        assert_eq!(text.read_all_bytes().unwrap(), PLAIN);
        assert_eq!(streams.load(Ordering::Relaxed), 1);
    }
}

/// `Holder::from_handle`: a second handle on the resource one addresses,
/// over the same store, touching nothing.
mod second_handles {
    use yggdryl::holder::{Buffer, Holder};
    use yggdryl::local::LocalFolder;
    use yggdryl::{Codec, IOBase, Level};

    #[test]
    fn a_local_role_is_held_again_over_its_path_beneath_every_wrapper() {
        let path = LocalFolder::temporary()
            .unwrap()
            .path()
            .unwrap()
            .join(format!("yggdryl-holder-second-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&path);
        let folder = Holder::folder(&path).unwrap();
        let mut leaf = folder.child_by_path("trades.bin").unwrap();
        leaf.write_all_bytes(b"AAPL").unwrap();

        let again = Holder::from_handle(&leaf).unwrap();
        assert_eq!(again.url(), leaf.url());
        assert_eq!(again.read_all_bytes().unwrap(), b"AAPL");
        let again = Holder::from_handle(&folder).unwrap();
        assert!(again.is_container());
        assert_eq!(again.url(), folder.url());

        // A coding is the caller's to compose again: the plain bytes beneath
        // it are what is held, under the media type they declare.
        let coded = Holder::from_handle(&leaf)
            .unwrap()
            .into_coded_with(Codec::Gzip, Level::default());
        let plain = Holder::from_handle(&coded).unwrap();
        assert!(!matches!(plain, Holder::Coded(_)), "{plain:?}");
        assert_eq!(plain.read_all_bytes().unwrap(), b"AAPL");
        let _ = std::fs::remove_dir_all(&path);
    }

    #[test]
    fn a_buffer_has_no_resource_to_hold_again() {
        let buffer = Holder::buffer(Buffer::from_bytes(b"AAPL".to_vec()));
        let error = Holder::from_handle(&buffer).unwrap_err();
        assert!(error.is_unsupported(), "{error}");
        assert!(error.to_string().contains("in-memory buffer"), "{error}");
    }
}

/// The object-store roles: what `Holder::from_url` holds a location of the
/// object stores' claimed backend as, what it reads off the location's query,
/// beneath the caller's properties, and takes off the location, and what an
/// object-store role answers for itself through the `Holder` - each a stated
/// number of requests.
#[cfg(feature = "s3")]
mod object_store_holders {
    use yggdryl::holder::Holder;
    use yggdryl::s3::{self, Credentials, S3File, S3Folder, S3Options, S3Path};
    use yggdryl::{Codec, IOBase, MimeType, Url};

    use crate::server::FakeS3;

    const NONE: [(&str, &str); 0] = [];

    /// `text` as one query value: the escapes a URL needs.
    fn escaped(text: &str) -> String {
        text.replace('%', "%25")
            .replace(':', "%3A")
            .replace('/', "%2F")
    }

    fn store() -> FakeS3 {
        let store = FakeS3::start();
        store.create_bucket("trades");
        store.allow_anonymous(true);
        store
    }

    /// `key` in the `trades` bucket, its query stating `store` as the
    /// endpoint the reader reaches anonymously.
    fn located(store: &FakeS3, key: &str) -> Url {
        Url::from_str(&format!(
            "s3://trades/{key}?endpoint_override={}&region=us-east-1&path_style=true&anonymous=true",
            escaped(&store.endpoint())
        ))
        .unwrap()
    }

    /// The methods and keys of the requests recorded since the last clear,
    /// a listing as a `GET` of no key.
    fn asked(store: &FakeS3) -> Vec<(String, Option<String>)> {
        store
            .requests()
            .into_iter()
            .map(|request| (request.method, request.key))
            .collect()
    }

    /// A source that answers in pieces smaller than it is asked for, and
    /// remembers the most it was ever asked for at once - the buffer an
    /// upload holds.
    struct Metered<'bytes> {
        bytes: &'bytes [u8],
        position: usize,
        largest_ask: usize,
    }

    impl std::io::Read for Metered<'_> {
        fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
            self.largest_ask = self.largest_ask.max(buffer.len());
            let length = buffer
                .len()
                .min(self.bytes.len() - self.position)
                .min(100 * 1024);
            buffer[..length].copy_from_slice(&self.bytes[self.position..self.position + length]);
            self.position += length;
            Ok(length)
        }
    }

    /// An object-store location is the claimed backend's undecided location,
    /// held as every claimed backend's handle is and described as every
    /// backend's is: what its name says the bytes are, or what `media_type`
    /// declares, and the coding `codec` names presented decoded - holding
    /// any of them sending nothing.
    #[test]
    fn an_object_store_location_is_a_registered_handle_described_as_any_other() {
        let store = store();

        let held = Holder::from_url(located(&store, "lake/part.parquet"), NONE).unwrap();
        assert!(matches!(held, Holder::Registered(_)), "{held:?}");
        let path = held
            .downcast_ref::<S3Path>()
            .expect("an undecided location");
        assert_eq!(path.key(), "lake/part.parquet");
        assert_eq!(held.media_type().base(), &MimeType::PARQUET);

        let declared = Holder::from_url(
            located(&store, "lake/part.bin"),
            [("media_type", "application/vnd.apache.parquet")],
        )
        .unwrap();
        assert!(declared.downcast_ref::<S3Path>().is_some(), "{declared:?}");
        assert_eq!(declared.media_type().base(), &MimeType::PARQUET);

        let mut coded =
            Holder::from_url(located(&store, "lake/part.csv"), [("codec", "gzip")]).unwrap();
        assert!(matches!(coded, Holder::Coded(_)), "{coded:?}");
        assert_eq!(store.request_count(), 0, "holding sends nothing");

        // The coding is the handle's: a write stores the value coded, and a
        // read presents it decoded.
        coded
            .write_all_bytes(b"symbol,price\nAAPL,187.23\n")
            .unwrap();
        let stored = store.get("trades", "lake/part.csv").expect("the object");
        assert_eq!(
            Codec::Gzip.load(&stored).unwrap(),
            b"symbol,price\nAAPL,187.23\n"
        );
        assert_eq!(
            coded.read_all_bytes().unwrap(),
            b"symbol,price\nAAPL,187.23\n"
        );
    }

    #[test]
    fn a_query_states_the_store_beneath_the_properties_and_leaves_the_location() {
        let store = store();
        store.put("trades", "lake/part.bin", b"AAPL");

        // The query names the store in the reader's own names and in
        // PyArrow's; the handle reports the location without it.
        let held = Holder::from_url(located(&store, "lake/part.bin"), NONE).unwrap();
        assert!(held.downcast_ref::<S3Path>().is_some(), "{held:?}");
        assert_eq!(
            held.url().map(ToString::to_string).as_deref(),
            Some("s3://trades/lake/part.bin")
        );
        assert_eq!(store.request_count(), 0, "holding sends nothing");
        assert_eq!(held.read_all_bytes().unwrap(), b"AAPL");
        assert!(
            store.request_count() > 0,
            "the read went to the endpoint the query named"
        );

        // A property the caller states wins over the query's, whatever its
        // spelling: the request goes where the property says.
        let url = Url::from_str(
            "s3://trades/lake/part.bin?endpoint_override=http%3A%2F%2Fexample.invalid%3A9&region=us-east-1&path_style=true&anonymous=true",
        )
        .unwrap();
        let held = Holder::from_url(&url, [("endpoint", store.endpoint().as_str())]).unwrap();
        store.clear_requests();
        assert_eq!(held.read_all_bytes().unwrap(), b"AAPL");
        assert!(store.request_count() > 0);

        // A parameter naming no property the store reads is refused by name
        // before anything is held, since an object takes no query.
        let url = Url::from_str("s3://trades/lake/part.bin?versionId=3").unwrap();
        let error = Holder::from_url(&url, NONE).unwrap_err();
        assert_eq!(
            error.to_string(),
            "invalid storage location expression at byte 0: the query parameter \"versionId\" \
             names no property the `s3` backend reads"
        );
    }

    /// Whether anything is at a location is the role's own answer; a role
    /// keeps what the store said, so a second handle asks again.
    #[test]
    fn a_store_role_answers_whether_anything_is_there() {
        let store = store();
        let held = Holder::from_url(located(&store, "lake/part.bin"), NONE).unwrap();
        assert!(!held.exists());
        store.put("trades", "lake/part.bin", b"AAPL");
        let again = Holder::from_url(located(&store, "lake/part.bin"), NONE).unwrap();
        assert!(again.exists());
    }

    /// A second handle on an object-store role is built on the role's own
    /// client - the endpoint, the key pair and every other option it was
    /// built with - with nothing sent: a folder cloned, a location and an
    /// object built as the child of their parent folder, each role kept.
    #[test]
    fn a_store_role_is_held_again_on_its_own_client_sending_nothing() {
        let store = FakeS3::start();
        store.create_bucket("trades");
        store.put("trades", "lake/part.bin", b"AAPL");
        let options = S3Options::default()
            .with_environment(false)
            .with_endpoint(store.endpoint())
            .with_region("us-east-1")
            .with_path_style(true)
            .with_credentials(Credentials::new("AKIAIOSFODNN7EXAMPLE", "wJalrXUtnFEMI"));
        let folder = Holder::from(s3::folder_with("s3://trades/lake", options).unwrap());
        let path = folder.child_by_path("part.bin").unwrap();
        let location = path
            .downcast_ref::<S3Path>()
            .unwrap_or_else(|| panic!("expected a location, got {path:?}"));
        let file = Holder::from(location.as_file().unwrap());
        store.clear_requests();

        let folder_again = Holder::from_handle(&folder).unwrap();
        let path_again = Holder::from_handle(&path).unwrap();
        let file_again = Holder::from_handle(&file).unwrap();
        assert_eq!(store.request_count(), 0, "holding again sends nothing");
        assert!(
            folder_again.downcast_ref::<S3Folder>().is_some(),
            "{folder_again:?}"
        );
        assert!(
            path_again.downcast_ref::<S3Path>().is_some(),
            "{path_again:?}"
        );
        assert!(
            file_again.downcast_ref::<S3File>().is_some(),
            "{file_again:?}"
        );
        assert_eq!(file_again.url(), file.url());

        // The read goes to the fake's endpoint, signed with the key pair the
        // first handle was built with.
        assert_eq!(file_again.read_all_bytes().unwrap(), b"AAPL");
        let signed = store.requests().iter().all(|request| {
            request.headers.iter().any(|(name, value)| {
                name.eq_ignore_ascii_case("authorization")
                    && value.contains("Credential=AKIAIOSFODNN7EXAMPLE/")
            })
        });
        assert!(
            store.request_count() > 0 && signed,
            "{:?}",
            store.requests()
        );
    }

    /// What an object-store role specializes it answers through the
    /// `Holder`, as the role itself would: a location re-described as the
    /// object or the prefix it names, a stated length kept, a stage dropped
    /// - none of them a request.
    #[test]
    fn a_store_role_answers_its_capabilities_through_the_holder_sending_nothing() {
        let store = store();
        store.put("trades", "lake/part.bin", b"AAPL");
        let held = Holder::from_url(located(&store, "lake/part.bin"), NONE).unwrap();

        let mut leaf = held.as_leaf().unwrap().expect("the object");
        assert!(leaf.downcast_ref::<S3File>().is_some(), "{leaf:?}");
        let prefix = held.as_container().unwrap().expect("the prefix");
        assert!(prefix.downcast_ref::<S3Folder>().is_some(), "{prefix:?}");

        // The object is a leaf already, and the prefix a container.
        assert!(leaf.as_leaf().unwrap().is_none());
        assert!(prefix.as_container().unwrap().is_none());

        leaf.set_known_size(4);
        assert_eq!(leaf.size(), 4);
        // An object publishes whole values alone: a dropped stage leaves
        // nothing a caller must remove.
        assert!(leaf.discard().unwrap());
        assert_eq!(store.request_count(), 0, "none of them a request");

        assert_eq!(leaf.read_all_bytes().unwrap(), b"AAPL");
        assert_eq!(
            asked(&store),
            [("GET".to_owned(), Some("lake/part.bin".to_owned()))]
        );
    }

    /// An upload through the `Holder` is the object's own: above the
    /// multipart threshold each part is read from the source and sent before
    /// the next, so the source is never asked for more than a part - where
    /// the whole-value default would read it whole first.
    #[test]
    fn an_upload_through_the_holder_is_the_objects_own_multipart_upload() {
        let store = store();
        let part = 5 * 1024 * 1024;
        let options = S3Options::default()
            .with_environment(false)
            .with_endpoint(store.endpoint())
            .with_region("us-east-1")
            .with_path_style(true)
            .with_anonymous(true)
            .with_multipart_threshold(512 * 1024)
            .with_part_size(part as u64);
        let mut held =
            Holder::from(s3::file_with("s3://trades/lake/streamed.bin", options).unwrap());

        // Six mebibytes over five-mebibyte parts.
        let bytes = b"AAPL,187.23;MSFT".repeat(393_216);
        let mut source = Metered {
            bytes: &bytes,
            position: 0,
            largest_ask: 0,
        };
        store.clear_requests();
        held.upload_from(&mut source, bytes.len() as u64).unwrap();
        assert_eq!(
            store.request_count(),
            4,
            "two parts, the create and the complete"
        );
        assert_eq!(
            source.largest_ask, part,
            "one part at a time, never the whole"
        );
        assert_eq!(store.get("trades", "lake/streamed.bin"), Some(bytes));
        assert_eq!(store.open_uploads(), 0);
    }

    /// A location spelling a prefix streams the objects beneath it, each
    /// through the stream the object owns: one listing, then one `GET` per
    /// object whatever the batch, and nothing asked about an object before
    /// it is read.
    #[test]
    fn a_prefix_location_streams_one_get_per_object() {
        let store = store();
        store.put("trades", "logs/a.log", b"a1\na2\n");
        store.put("trades", "logs/b.log", b"b1\nb2");
        let held = Holder::from_url(located(&store, "logs/"), NONE).unwrap();

        store.clear_requests();
        let chunks = held
            .pstream_bytes(0, 4)
            .unwrap()
            .collect::<yggdryl::Result<Vec<_>>>()
            .unwrap();
        assert_eq!(chunks.concat(), b"a1\na2\nb1\nb2");
        assert_eq!(
            asked(&store),
            [
                ("GET".to_owned(), None),
                ("GET".to_owned(), Some("logs/a.log".to_owned())),
                ("GET".to_owned(), Some("logs/b.log".to_owned())),
            ]
        );
    }
}

/// The four HTTP variants: what `Holder::from_url` routes to an `http:` or
/// `https:` URL, the properties it reads, and what composes over one.
#[cfg(feature = "http")]
mod http_holders {
    use std::time::Duration;

    use yggdryl::holder::Holder;
    use yggdryl::http::{HttpOptions, Session};
    use yggdryl::{IOBase, IOKind, MimeType, Url};

    #[test]
    fn an_http_url_is_held_as_the_request_that_reads_it() {
        let url = Url::from_str("https://data.example.com/lake/trades.parquet").unwrap();
        let none: [(&str, &str); 0] = [];
        let held = Holder::from_url(&url, none).unwrap();
        let Holder::HttpRequest(request) = &held else {
            panic!("expected a request, got {held:?}");
        };
        assert_eq!(request.url(), &url);
        assert_eq!(held.url(), Some(&url));
        assert_eq!(held.media_type().base(), &MimeType::PARQUET);
        assert!(!held.is_container());
        // Holding costs nothing.
        assert_eq!(request.stats().requests, 0);
        let held: Holder = Holder::try_from(&url).unwrap();
        assert!(matches!(held, Holder::HttpRequest(_)));
    }

    #[test]
    fn the_properties_configure_the_session_and_the_two_generic_ones_apply() {
        let url = Url::from_str("http://api.example.com/v1/blob").unwrap();
        let held = Holder::from_url(
            &url,
            [
                ("timeout", "9"),
                ("max-attempts", "1"),
                ("header.X-Api-Key", "k-1"),
                ("bearer_token", "t-1"),
                ("media_type", "application/json"),
                ("warehouse", "ignored"),
            ],
        )
        .unwrap();
        let Holder::HttpRequest(request) = &held else {
            panic!("expected a request, got {held:?}");
        };
        let options = request.session().options();
        assert_eq!(options.timeout(), Duration::from_secs(9));
        assert_eq!(options.max_attempts(), 1);
        assert_eq!(options.headers().get("x-api-key"), Some("k-1"));
        assert!(options.authorization().is_some());
        assert_eq!(held.media_type().base(), &MimeType::JSON);

        let coded = Holder::from_url(&url, [("codec", "gzip")]).unwrap();
        assert!(matches!(coded, Holder::Coded(_)), "{coded:?}");
    }

    #[test]
    fn a_property_that_does_not_parse_is_refused_before_anything_is_held() {
        let url = Url::from_str("http://api.example.com/v1/blob").unwrap();
        let error = Holder::from_url(&url, [("timeout", "soon")]).expect_err("a refusal");
        assert!(
            matches!(
                error,
                yggdryl::Error::Parse {
                    target: "http option",
                    ..
                }
            ),
            "{error:?}"
        );
    }

    #[test]
    fn into_declared_media_composes_over_an_http_location_without_a_request() {
        let session = Session::with_options(HttpOptions::default()).unwrap();
        let request = session
            .get("http://data.example.com/logs/app.txt.gz")
            .unwrap();
        let composed = Holder::HttpRequest(request).into_declared_media();
        match &composed {
            Holder::Text(text) => match text.handle() {
                Holder::Coded(coded) => {
                    assert!(matches!(coded.handle(), Holder::HttpRequest(_)));
                }
                other => panic!("expected a coding under the text, got {other:?}"),
            },
            other => panic!("expected text over a coding, got {other:?}"),
        }
        assert_eq!(composed.media_type().base(), &MimeType::PLAIN_TEXT);
        assert_eq!(session.stats().requests, 0);

        let plain = Holder::HttpRequest(session.get("http://data.example.com/a.json").unwrap())
            .into_declared_media();
        assert!(matches!(plain, Holder::HttpRequest(_)), "{plain:?}");
    }

    #[test]
    fn a_session_is_held_as_a_container() {
        let base = Url::from_str("http://api.example.com/v1/").unwrap();
        let session =
            Session::with_options(HttpOptions::default().with_base_url(base.clone())).unwrap();
        let held = Holder::from(session);
        assert!(matches!(held, Holder::HttpSession(_)));
        assert_eq!(held.kind(), IOKind::Directory);
        assert_eq!(held.url(), Some(&base));
        let child = held.child_by_path("items").unwrap();
        assert!(matches!(child, Holder::HttpRequest(_)));
        assert_eq!(
            child.url().map(ToString::to_string).as_deref(),
            Some("http://api.example.com/v1/items")
        );
    }
}
