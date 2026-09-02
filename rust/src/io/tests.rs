//! Behavior every [`IOBase`] implementation must share.

use std::io::{Read, Write};

use super::{Buffer, IOBase};
use crate::Codec;
use crate::{Field, MediaType, MimeType, Scalar, Url};

#[test]
fn positional_writes_grow_and_zero_fill_the_gap() {
    let mut buffer = Buffer::new();
    assert!(buffer.is_empty());

    buffer.pwrite(0, b"trade").unwrap();
    assert_eq!(buffer.size(), 5);

    // Writing past the end grows the value and zero-fills what was skipped.
    buffer.pwrite(8, b"!").unwrap();
    assert_eq!(buffer.size(), 9);
    assert_eq!(buffer.as_slice(), b"trade\0\0\0!");
}

#[test]
fn positional_reads_do_not_share_a_cursor() {
    let buffer = Buffer::from_bytes(b"0123456789".to_vec());

    // Two independent reads at different offsets, in any order.
    let mut tail = [0_u8; 3];
    buffer.pread(7, &mut tail).unwrap();
    let mut head = [0_u8; 3];
    buffer.pread(0, &mut head).unwrap();

    assert_eq!(&head, b"012");
    assert_eq!(&tail, b"789");

    // A read entirely past the end is empty rather than an error.
    let mut past = [0_u8; 4];
    assert_eq!(buffer.pread(100, &mut past).unwrap(), 0);

    // A read straddling the end is short.
    assert_eq!(buffer.pread(8, &mut past).unwrap(), 2);
}

#[test]
fn exact_reads_name_the_shortfall() {
    let buffer = Buffer::from_bytes(b"abc".to_vec());
    let mut target = [0_u8; 8];
    let message = buffer.pread_exact(0, &mut target).unwrap_err().to_string();
    assert!(message.contains("expected 8 bytes"), "{message}");
    assert!(message.contains("got 3"), "{message}");
}

#[test]
fn truncate_shrinks_and_extends() {
    let mut buffer = Buffer::from_bytes(b"0123456789".to_vec());

    buffer.truncate(4).unwrap();
    assert_eq!(buffer.as_slice(), b"0123");

    // Extending zero-fills rather than leaving stale bytes visible.
    buffer.truncate(6).unwrap();
    assert_eq!(buffer.as_slice(), b"0123\0\0");

    buffer.clear().unwrap();
    assert!(buffer.is_empty());
}

#[test]
fn reserve_grows_capacity_without_changing_size() {
    let mut buffer = Buffer::new();
    buffer.reserve(4_096).unwrap();

    assert!(buffer.capacity() >= 4_096);
    assert_eq!(buffer.size(), 0);
    // Capacity is never below size, for every implementation.
    buffer.pwrite(0, b"x").unwrap();
    assert!(buffer.capacity() >= buffer.size());
}

#[test]
fn append_reports_where_the_bytes_landed() {
    let mut buffer = Buffer::new();
    assert_eq!(buffer.append_bytes(b"first").unwrap(), 0);
    assert_eq!(buffer.append_bytes(b"second").unwrap(), 5);
    assert_eq!(buffer.as_slice(), b"firstsecond");
}

#[test]
fn a_declared_media_type_overrides_inference() {
    let url = Url::from_str("file:///trades.json.gz").unwrap();
    let buffer = Buffer::new().with_media_type(url.media_type());

    assert_eq!(buffer.media_type().base(), &MimeType::JSON);
    assert_eq!(buffer.codec(), Codec::Gzip);

    // Setting one later replaces whatever was inferred.
    let mut plain = Buffer::from_bytes(b"{\"a\":1}".to_vec());
    assert_eq!(plain.media_type().base(), &MimeType::JSON);
    plain.set_media_type(MediaType::from(MimeType::CSV));
    assert_eq!(plain.media_type().base(), &MimeType::CSV);
}

#[test]
fn an_undeclared_media_type_is_inferred_from_content() {
    // A buffer has no filename, so its representation comes from its bytes.
    let json = Buffer::from_bytes(br#"{"symbol":"AAPL"}"#.to_vec());
    assert_eq!(json.media_type().base(), &MimeType::JSON);

    let parquet = Buffer::from_bytes(b"PAR1payload".to_vec());
    assert_eq!(parquet.media_type().base(), &MimeType::PARQUET);

    // Opaque bytes stay opaque rather than guessing.
    let opaque = Buffer::from_bytes(vec![0xAB, 0xCD, 0xEF]);
    assert_eq!(opaque.media_type().base(), &MimeType::OCTET_STREAM);
    assert!(Buffer::new().media_type().base() == &MimeType::OCTET_STREAM);
}

#[test]
fn structured_values_follow_the_declared_format_and_content_coding() {
    let expected = Scalar::from_record([
        ("quantity", Scalar::I64(2)),
        ("symbol", Scalar::from("AAPL")),
    ])
    .unwrap();

    for name in [
        "trade.json",
        "trade.json.gz",
        "trade.json.zz",
        "trade.json.zst",
        "trade.yaml",
        "trade.toml",
    ] {
        let media = Url::from_str(&format!("file:///{name}"))
            .unwrap()
            .media_type();
        let mut handle = Buffer::new().with_media_type(media);
        handle
            .write_scalar(&expected)
            .unwrap_or_else(|error| panic!("{name}: {error}"));
        let actual = handle
            .read_scalar(None)
            .unwrap_or_else(|error| panic!("{name}: {error}"));
        assert_eq!(actual, expected, "{name}");
    }
}

#[test]
fn structured_value_fields_direct_parsing_and_casting() {
    let media = Url::from_str("file:///trade.json").unwrap().media_type();
    let source = Buffer::from_bytes(br#"{"quantity":2}"#.to_vec()).with_media_type(media);
    let field = Field::from_str("trade: struct<quantity: int32 not null> not null").unwrap();
    let expected = Scalar::from_sequence([Scalar::I64(2)]);

    assert_eq!(source.read_scalar(Some(&field)).unwrap(), expected);

    let invalid = Buffer::from_bytes(br#"{"quantity":"many"}"#.to_vec())
        .with_media_type(Url::from_str("file:///trade.json").unwrap().media_type());
    let message = invalid.read_scalar(Some(&field)).unwrap_err().to_string();
    assert!(message.contains("quantity"), "{message}");
    assert!(message.contains("int32"), "{message}");
}

#[test]
fn inference_is_redone_after_the_bytes_change() {
    let mut buffer = Buffer::from_bytes(br#"{"a":1}"#.to_vec());
    assert_eq!(buffer.media_type().base(), &MimeType::JSON);

    // Replacing the content replaces the inferred representation.
    buffer.write_all_bytes(b"PAR1payload").unwrap();
    assert_eq!(buffer.media_type().base(), &MimeType::PARQUET);

    buffer.clear().unwrap();
    assert_eq!(buffer.media_type().base(), &MimeType::OCTET_STREAM);
}

#[test]
fn a_buffer_reports_a_mem_identity_rather_than_a_location() {
    let buffer = Buffer::from_bytes(b"bytes".to_vec());
    let identity = buffer.url().expect("a buffer always has an identity");

    // The bytes are not stored anywhere, so this names the process and the
    // allocation rather than a place on disk.
    assert_eq!(identity.scheme().as_str(), "mem");
    assert_eq!(
        identity.authority().as_str(),
        std::process::id().to_string()
    );
    assert!(identity.path().as_str().contains("0x"), "{identity}");

    // The identity is stable for one handle.
    assert_eq!(buffer.url(), Some(identity));

    // A distinct buffer is distinguishable from it.
    let other = Buffer::from_bytes(b"bytes".to_vec());
    assert_ne!(other.url(), Some(identity));
}

#[test]
fn copy_into_moves_bytes_and_media_type() {
    let source = Buffer::from_bytes(b"symbol,price\nAAPL,1\n".to_vec())
        .with_media_type(Url::from_str("file:///trades.csv").unwrap().media_type());
    let mut target = Buffer::from_bytes(b"stale contents".to_vec());

    let copied = source.copy_into(&mut target).unwrap();

    assert_eq!(copied, source.size());
    assert_eq!(target.as_slice(), source.as_slice());
    assert_eq!(target.media_type().base(), &MimeType::CSV);
}

#[test]
fn compression_round_trips_and_tracks_the_coding() {
    let payload = "symbol,price\n".repeat(500).into_bytes();
    let source = Buffer::from_bytes(payload.clone())
        .with_media_type(Url::from_str("file:///trades.csv").unwrap().media_type());

    for codec in [Codec::Gzip, Codec::Zlib, Codec::Zstd] {
        let mut compressed = Buffer::new();
        let written = source.compress_into(&mut compressed, codec).unwrap();

        assert_eq!(written, compressed.size());
        assert!(compressed.size() < source.size(), "{codec}");
        // The target records the coding, so decoding needs no extra argument.
        assert_eq!(compressed.codec(), codec, "{codec}");
        assert_eq!(compressed.media_type().base(), &MimeType::CSV, "{codec}");

        let mut restored = Buffer::new();
        compressed.decompress_into(&mut restored).unwrap();
        assert_eq!(restored.as_slice(), payload.as_slice(), "{codec}");
        // The coding is gone once decoded.
        assert_eq!(restored.codec(), Codec::Identity, "{codec}");
    }
}

#[test]
fn streaming_adapters_advance_their_own_offset() {
    let mut buffer = Buffer::new();
    {
        let mut writer = buffer.writer_at(0);
        writer.write_all(b"symbol,").unwrap();
        writer.write_all(b"price").unwrap();
        writer.flush().unwrap();
    }
    assert_eq!(buffer.as_slice(), b"symbol,price");

    let mut text = String::new();
    buffer.reader_at(0).read_to_string(&mut text).unwrap();
    assert_eq!(text, "symbol,price");

    // A reader can start anywhere without disturbing another.
    let mut tail = String::new();
    buffer.reader_at(7).read_to_string(&mut tail).unwrap();
    assert_eq!(tail, "price");
}

#[test]
fn read_range_is_bounded_by_the_value() {
    let buffer = Buffer::from_bytes(b"0123456789".to_vec());
    assert_eq!(buffer.read_range_bytes(2, 3).unwrap(), b"234");
    // Asking past the end yields what exists rather than failing.
    assert_eq!(buffer.read_range_bytes(8, 100).unwrap(), b"89");
    assert!(buffer.read_range_bytes(50, 4).unwrap().is_empty());
}

#[test]
fn write_all_bytes_replaces_the_whole_value() {
    let mut buffer = Buffer::from_bytes(b"a much longer previous value".to_vec());
    buffer.write_all_bytes(b"short").unwrap();
    assert_eq!(buffer.as_slice(), b"short");
    assert_eq!(buffer.size(), 5);
}

#[test]
fn boxed_cursors_preserve_lifecycle_hierarchy_and_kind() {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};

    use crate::generic::Holder;
    use crate::io::{IOMedia, Listing};
    use crate::{IOKind, Result};

    struct Probe {
        bytes: Buffer,
        opened: Arc<AtomicBool>,
    }

    impl IOMedia for Probe {
        crate::impl_default_iomedia!();
    }

    impl IOBase for Probe {
        crate::delegate_iobase!(bytes: pread, pstream_bytes, pwrite, size, capacity, reserve,
            truncate, url, media_type, set_media_type, flush, clear, remove);

        fn open(&mut self) -> Result<()> {
            self.opened.store(true, Ordering::SeqCst);
            Ok(())
        }

        fn opened(&self) -> bool {
            self.opened.load(Ordering::SeqCst)
        }

        fn close(&mut self) -> Result<()> {
            self.opened.store(false, Ordering::SeqCst);
            Ok(())
        }

        fn parent(&self) -> Option<Holder> {
            Some(Holder::buffer(Buffer::from_bytes(b"parent".to_vec())))
        }

        fn child_by_path(&self, path: &str) -> Result<Holder> {
            Ok(Holder::buffer(Buffer::from_bytes(path.as_bytes().to_vec())))
        }

        fn ls(&self, recursive: bool, include_private: bool) -> Listing {
            let value = format!("{recursive}:{include_private}");
            Listing::new(std::iter::once(Ok(Holder::buffer(Buffer::from_bytes(
                value.into_bytes(),
            )))))
        }

        fn kind(&self) -> IOKind {
            IOKind::Directory
        }
    }

    let state = Arc::new(AtomicBool::new(false));
    let mut handle: Box<dyn IOBase> = Box::new(crate::io::Cursor::new(Probe {
        bytes: Buffer::new(),
        opened: Arc::clone(&state),
    }));

    assert_eq!(handle.kind(), IOKind::Directory);
    assert!(handle.is_container());
    assert_eq!(
        handle.parent().unwrap().read_all_bytes().unwrap(),
        b"parent"
    );
    assert_eq!(
        handle
            .child_by_path("nested/leaf")
            .unwrap()
            .read_all_bytes()
            .unwrap(),
        b"nested/leaf"
    );
    assert_eq!(
        handle
            .ls(true, true)
            .next()
            .unwrap()
            .unwrap()
            .read_all_bytes()
            .unwrap(),
        b"true:true"
    );

    handle.open().unwrap();
    assert!(handle.opened());
    assert!(state.load(Ordering::SeqCst));
    handle.close().unwrap();
    assert!(handle.closed());
    assert!(!state.load(Ordering::SeqCst));
}

/// Any handle reads through one reader and writes through three explicit
/// intents. Held record batches are zero-copy adapters over those primitives.
#[cfg(feature = "arrow")]
mod records {
    use std::sync::Arc;
    use std::sync::Mutex;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use arrow_array::{Int64Array, RecordBatch, RecordBatchReader, StringArray};
    use arrow_schema::{ArrowError, SchemaRef};

    use crate::arrow::BatchReader;
    use crate::generic::{IORecordOptions, RecordOptions};
    use crate::io::{ArrowWriteSession, Buffer, IOBase, IOMedia};
    use crate::{DataType, Error, Field, IOMode, MimeType, Scalar, Url};

    fn schema() -> Field {
        DataType::from_fields([
            DataType::Int64.required_field("id"),
            DataType::Utf8.nullable_field("symbol"),
        ])
        .unwrap()
        .required_field("row")
    }

    fn batch() -> RecordBatch {
        RecordBatch::try_new(
            crate::arrow::arrow_schema_from_field(&schema()).unwrap(),
            vec![
                Arc::new(Int64Array::from(vec![1, 2])),
                Arc::new(StringArray::from(vec![Some("AAPL"), None])),
            ],
        )
        .unwrap()
    }

    /// The batches a write takes: one reader over one two-row batch.
    fn reader() -> BatchReader {
        crate::arrow::batch_reader(
            crate::arrow::arrow_schema_from_field(&schema()).unwrap(),
            [batch()],
        )
    }

    fn handle(name: &str) -> Buffer {
        Buffer::new().with_media_type(
            Url::from_str(&format!("file:///{name}"))
                .unwrap()
                .media_type(),
        )
    }

    /// The total row count a handle currently holds.
    fn rows(handle: &impl IOBase, options: &RecordOptions) -> usize {
        handle
            .read_arrow_reader(options)
            .unwrap()
            .map(|batch| batch.unwrap().num_rows())
            .sum()
    }

    fn rows_batch(ids: &[i64]) -> RecordBatch {
        RecordBatch::try_new(
            crate::arrow::arrow_schema_from_field(&schema()).unwrap(),
            vec![
                Arc::new(Int64Array::from(ids.to_vec())),
                Arc::new(StringArray::from(
                    ids.iter().map(|_| Some("S")).collect::<Vec<_>>(),
                )),
            ],
        )
        .unwrap()
    }

    /// A byte handle that observes each complete encoded publication.
    struct PublicationProbe {
        handle: Buffer,
        publications: Arc<AtomicUsize>,
        source_pulls: Arc<AtomicUsize>,
        pulls_when_published: Arc<Mutex<Vec<usize>>>,
        destination_touches: Arc<AtomicUsize>,
        fail_publication: Option<usize>,
    }

    impl PublicationProbe {
        fn new(name: &str, source_pulls: Arc<AtomicUsize>) -> Self {
            Self {
                handle: handle(name),
                publications: Arc::new(AtomicUsize::new(0)),
                source_pulls,
                pulls_when_published: Arc::new(Mutex::new(Vec::new())),
                destination_touches: Arc::new(AtomicUsize::new(0)),
                fail_publication: None,
            }
        }

        fn reset_publications(&self) {
            self.publications.store(0, Ordering::SeqCst);
            self.pulls_when_published.lock().unwrap().clear();
        }

        fn fail_on_publication(&mut self, publication: usize) {
            self.fail_publication = Some(publication);
        }
    }

    impl crate::io::IOMedia for PublicationProbe {
        crate::impl_default_iomedia!();
    }

    impl IOBase for PublicationProbe {
        fn pread(&self, offset: u64, buffer: &mut [u8]) -> crate::Result<usize> {
            self.handle.pread(offset, buffer)
        }

        fn pwrite(&mut self, offset: u64, bytes: &[u8]) -> crate::Result<usize> {
            self.handle.pwrite(offset, bytes)
        }

        fn size(&self) -> u64 {
            self.handle.size()
        }

        fn capacity(&self) -> u64 {
            self.handle.capacity()
        }

        fn reserve(&mut self, capacity: u64) -> crate::Result<()> {
            self.handle.reserve(capacity)
        }

        fn truncate(&mut self, size: u64) -> crate::Result<()> {
            self.handle.truncate(size)
        }

        fn url(&self) -> Option<&Url> {
            self.handle.url()
        }

        fn media_type(&self) -> &crate::MediaType {
            self.handle.media_type()
        }

        fn set_media_type(&mut self, media_type: crate::MediaType) {
            self.handle.set_media_type(media_type);
        }

        fn kind(&self) -> crate::IOKind {
            self.destination_touches.fetch_add(1, Ordering::SeqCst);
            crate::IOKind::Memory
        }

        fn write_all_bytes(&mut self, bytes: &[u8]) -> crate::Result<()> {
            let publication = self.publications.fetch_add(1, Ordering::SeqCst) + 1;
            self.pulls_when_published
                .lock()
                .unwrap()
                .push(self.source_pulls.load(Ordering::SeqCst));
            if self.fail_publication == Some(publication) {
                return Err(crate::Error::Io(std::io::Error::other(format!(
                    "publication {publication} refused"
                ))));
            }
            self.handle.write_all_bytes(bytes)
        }
    }

    /// A fallible source whose exact pull frontier is observable.
    struct CountedSource {
        schema: SchemaRef,
        batches: std::collections::VecDeque<std::result::Result<RecordBatch, ArrowError>>,
        pulls: Arc<AtomicUsize>,
    }

    impl Iterator for CountedSource {
        type Item = std::result::Result<RecordBatch, ArrowError>;

        fn next(&mut self) -> Option<Self::Item> {
            let item = self.batches.pop_front()?;
            self.pulls.fetch_add(1, Ordering::SeqCst);
            Some(item)
        }
    }

    impl RecordBatchReader for CountedSource {
        fn schema(&self) -> SchemaRef {
            Arc::clone(&self.schema)
        }
    }

    fn counted_source(
        pulls: Arc<AtomicUsize>,
        batches: impl IntoIterator<Item = std::result::Result<RecordBatch, ArrowError>>,
    ) -> BatchReader {
        Box::new(CountedSource {
            schema: crate::arrow::arrow_schema_from_field(&schema()).unwrap(),
            batches: batches.into_iter().collect(),
            pulls,
        })
    }

    #[test]
    fn the_handles_media_type_picks_the_record_encoding() {
        assert!(matches!(
            handle("t.arrows").record_options().unwrap(),
            RecordOptions::Ipc(_)
        ));
        #[cfg(feature = "parquet")]
        assert!(matches!(
            handle("t.parquet").record_options().unwrap(),
            RecordOptions::Parquet(_)
        ));

        // An encoding with no implementation is named rather than guessed.
        let message = handle("t.csv").record_options().unwrap_err().to_string();
        assert!(message.contains("text/csv"), "{message}");
    }

    #[test]
    fn batches_round_trip_through_a_bare_handle() {
        let mut names = vec!["t.arrows", "t.arrows.zst"];
        if cfg!(feature = "parquet") {
            names.push("t.parquet");
        }

        for name in names {
            let mut handle = handle(name);
            let options = handle
                .record_options()
                .unwrap()
                .with_field(schema())
                .with_safe(true);

            handle
                .overwrite_arrow_reader(reader(), &options)
                .unwrap_or_else(|error| panic!("{name}: {error}"));
            assert_eq!(rows(&handle, &options), 2, "{name}");
            assert_eq!(
                handle.read_arrow_field(&options).unwrap(),
                schema(),
                "{name}"
            );
        }
    }

    #[test]
    fn a_write_stores_the_schema_its_reader_declares() {
        let mut handle = handle("declared.arrows");
        let options = handle.record_options().unwrap();

        // The write path takes a reader and nothing else, so with nothing
        // declared and nothing stored the reader's own schema is what the
        // resource ends up holding.
        handle.overwrite_arrow_reader(reader(), &options).unwrap();

        assert_eq!(handle.read_arrow_field(&options).unwrap(), schema());
        assert_eq!(
            handle
                .read_arrow_reader(&options)
                .unwrap()
                .schema()
                .fields()
                .len(),
            2
        );
    }

    #[test]
    fn an_overwrite_keeps_the_schema_the_resource_already_stores() {
        let mut handle = handle("stable.arrows");
        let options = handle.record_options().unwrap();
        handle.overwrite_arrow_reader(reader(), &options).unwrap();

        // The incoming rows declare `id` as text and drop `symbol` entirely. An
        // overwrite replaces rows, so the stored columns survive it and the
        // text is cast back into the stored Int64.
        let loose = DataType::from_fields([DataType::Utf8.required_field("id")])
            .unwrap()
            .required_field("row");
        let incoming = RecordBatch::try_new(
            crate::arrow::arrow_schema_from_field(&loose).unwrap(),
            vec![Arc::new(StringArray::from(vec!["7"]))],
        )
        .unwrap();
        handle
            .overwrite_arrow_reader(
                crate::arrow::batch_reader(incoming.schema(), [incoming]),
                &options,
            )
            .unwrap();

        assert_eq!(handle.read_arrow_field(&options).unwrap(), schema());
        assert_eq!(rows(&handle, &options), 1);
    }

    #[test]
    fn a_missing_resource_reads_as_empty_rather_than_failing() {
        let handle = Buffer::new().with_media_type(MimeType::ARROW_STREAM.into());
        let options = handle.record_options().unwrap().with_field(schema());

        assert_eq!(handle.read_arrow_reader(&options).unwrap().count(), 0);
        assert_eq!(handle.read_arrow_field(&options).unwrap(), schema());
    }

    #[test]
    fn appending_reads_adds_and_rewrites() {
        let mut handle = handle("append.arrows");
        let options = handle.record_options().unwrap().with_field(schema());

        // Appending to nothing simply writes.
        handle.append_arrow_reader(reader(), &options).unwrap();
        assert_eq!(rows(&handle, &options), 2);

        handle.append_arrow_reader(reader(), &options).unwrap();
        assert_eq!(rows(&handle, &options), 4);
    }

    #[test]
    fn commit_row_size_controls_exact_publication_counts() {
        for (label, cadence, expected) in [
            ("unset", None, 1),
            ("one", Some(1), 4),
            ("across-batches", Some(3), 2),
            ("larger-than-stream", Some(10), 1),
        ] {
            let pulls = Arc::new(AtomicUsize::new(0));
            let mut handle = PublicationProbe::new(
                &format!("commit-publications-{label}.arrows"),
                Arc::clone(&pulls),
            );
            let mut options = handle.record_options().unwrap().with_field(schema());
            options.set_commit_row_size(cadence);
            let source = crate::arrow::batch_reader(
                crate::arrow::arrow_schema_from_field(&schema()).unwrap(),
                [rows_batch(&[1, 2]), rows_batch(&[3, 4])],
            );

            handle.overwrite_arrow_reader(source, &options).unwrap();

            assert_eq!(
                handle.publications.load(Ordering::SeqCst),
                expected,
                "{label}"
            );
            assert_eq!(rows(&handle, &options), 4, "{label}");
        }
    }

    #[test]
    fn every_write_intent_retains_its_intent_for_each_commit() {
        for intent in ["overwrite", "append", "merge"] {
            let pulls = Arc::new(AtomicUsize::new(0));
            let mut handle = PublicationProbe::new(
                &format!("commit-intent-{intent}.arrows"),
                Arc::clone(&pulls),
            );
            let plain = handle.record_options().unwrap().with_field(schema());
            handle
                .overwrite_arrow_reader(
                    crate::arrow::batch_reader(
                        crate::arrow::arrow_schema_from_field(&schema()).unwrap(),
                        [rows_batch(&[1, 2])],
                    ),
                    &plain,
                )
                .unwrap();
            handle.reset_publications();

            let options = plain.clone().with_commit_row_size(2);
            let incoming = crate::arrow::batch_reader(
                crate::arrow::arrow_schema_from_field(&schema()).unwrap(),
                [rows_batch(&[1, 3, 4, 5])],
            );
            match intent {
                "overwrite" => handle.overwrite_arrow_reader(incoming, &options).unwrap(),
                "append" => handle.append_arrow_reader(incoming, &options).unwrap(),
                "merge" => handle
                    .merge_arrow_reader(incoming, &options.with_merge_by_names(["id"]))
                    .unwrap(),
                _ => unreachable!(),
            }

            assert_eq!(handle.publications.load(Ordering::SeqCst), 2, "{intent}");
            let expected_rows = match intent {
                "overwrite" => 4,
                "append" => 6,
                "merge" => 5,
                _ => unreachable!(),
            };
            assert_eq!(rows(&handle, &plain), expected_rows, "{intent}");
        }
    }

    #[test]
    fn held_batch_and_native_row_adapters_inherit_commit_boundaries() {
        let pulls = Arc::new(AtomicUsize::new(0));
        let mut batch_handle = PublicationProbe::new("commit-batch.arrows", Arc::clone(&pulls));
        let options = batch_handle
            .record_options()
            .unwrap()
            .with_field(schema())
            .with_commit_row_size(1);
        batch_handle
            .overwrite_arrow_batch(rows_batch(&[1, 2]), &options)
            .unwrap();
        assert_eq!(batch_handle.publications.load(Ordering::SeqCst), 2);

        let mut row_handle = PublicationProbe::new("commit-rows.arrows", pulls);
        row_handle
            .overwrite_records(
                [
                    NativeRow {
                        id: 1,
                        symbol: Some("AAPL"),
                    },
                    NativeRow {
                        id: 2,
                        symbol: Some("MSFT"),
                    },
                ],
                &options,
            )
            .unwrap();
        assert_eq!(row_handle.publications.load(Ordering::SeqCst), 2);
        assert_eq!(rows(&row_handle, &options), 2);
    }

    #[test]
    fn zero_commit_row_size_is_rejected_before_any_input_pull() {
        for intent in ["overwrite", "append", "merge"] {
            let pulls = Arc::new(AtomicUsize::new(0));
            let mut handle =
                PublicationProbe::new(&format!("zero-commit-{intent}.arrows"), Arc::clone(&pulls));
            let options = handle
                .record_options()
                .unwrap()
                .with_field(schema())
                .with_commit_row_size(0);
            let source = counted_source(Arc::clone(&pulls), [Ok(rows_batch(&[1]))]);
            let result = match intent {
                "overwrite" => handle.overwrite_arrow_reader(source, &options),
                "append" => handle.append_arrow_reader(source, &options),
                "merge" => {
                    handle.merge_arrow_reader(source, &options.clone().with_merge_by_names(["id"]))
                }
                _ => unreachable!(),
            };

            let message = result.unwrap_err().to_string();
            assert!(message.contains("commit_row_size"), "{intent}: {message}");
            assert_eq!(pulls.load(Ordering::SeqCst), 0, "{intent}");
            assert_eq!(handle.publications.load(Ordering::SeqCst), 0, "{intent}");
        }

        let pulls = Arc::new(AtomicUsize::new(0));
        let counted = Arc::clone(&pulls);
        let records = std::iter::from_fn(move || {
            counted.fetch_add(1, Ordering::SeqCst);
            Some(NativeRow {
                id: 1,
                symbol: None,
            })
        });
        let mut handle = handle("zero-commit-native.arrows");
        let options = handle
            .record_options()
            .unwrap()
            .with_field(schema())
            .with_commit_row_size(0);
        let message = handle
            .overwrite_records(records, &options)
            .unwrap_err()
            .to_string();
        assert!(message.contains("commit_row_size"), "{message}");
        assert_eq!(pulls.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn empty_append_and_merge_do_not_touch_the_destination() {
        let source_pulls = Arc::new(AtomicUsize::new(0));
        let mut handle = PublicationProbe::new("empty-no-touch.arrows", source_pulls);
        let options = handle.record_options().unwrap().with_field(schema());
        let touches = Arc::clone(&handle.destination_touches);
        // Option discovery is outside the write; count only destination work
        // performed after the empty source crosses the primitive boundary.
        touches.store(0, Ordering::SeqCst);
        let empty = || {
            crate::arrow::batch_reader(
                crate::arrow::arrow_schema_from_field(&schema()).unwrap(),
                [],
            )
        };

        handle.append_arrow_reader(empty(), &options).unwrap();
        handle
            .merge_arrow_reader(empty(), &options.with_merge_by_names(["id"]))
            .unwrap();

        assert_eq!(touches.load(Ordering::SeqCst), 0);
        assert_eq!(handle.publications.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn zero_append_limits_and_invalid_merge_limits_do_not_pull() {
        for options in [
            handle("limit-options.arrows")
                .record_options()
                .unwrap()
                .with_field(schema())
                .with_max_row_size(0),
            handle("limit-options.arrows")
                .record_options()
                .unwrap()
                .with_field(schema())
                .with_max_byte_size(0),
        ] {
            let pulls = Arc::new(AtomicUsize::new(0));
            let source = counted_source(Arc::clone(&pulls), [Ok(rows_batch(&[1]))]);
            let mut destination = handle("zero-limit-append.arrows");
            destination.append_arrow_reader(source, &options).unwrap();
            assert_eq!(pulls.load(Ordering::SeqCst), 0);
        }

        for limited in [
            handle("merge-limit-options.arrows")
                .record_options()
                .unwrap()
                .with_field(schema())
                .with_merge_by_names(["id"])
                .with_max_row_size(1),
            handle("merge-limit-options.arrows")
                .record_options()
                .unwrap()
                .with_field(schema())
                .with_merge_by_names(["id"])
                .with_max_byte_size(1),
        ] {
            let pulls = Arc::new(AtomicUsize::new(0));
            let source = counted_source(Arc::clone(&pulls), [Ok(rows_batch(&[1]))]);
            let mut destination = handle("invalid-limit-merge.arrows");
            let message = destination
                .merge_arrow_reader(source, &limited)
                .unwrap_err()
                .to_string();
            assert!(message.contains("merge_by_names"), "{message}");
            assert_eq!(pulls.load(Ordering::SeqCst), 0);
        }
    }

    #[test]
    fn a_later_source_failure_leaves_each_successful_prefix_visible() {
        for intent in ["overwrite", "append", "merge"] {
            let pulls = Arc::new(AtomicUsize::new(0));
            let mut handle = PublicationProbe::new(
                &format!("partial-commit-{intent}.arrows"),
                Arc::clone(&pulls),
            );
            let plain = handle.record_options().unwrap().with_field(schema());
            if intent != "overwrite" {
                handle
                    .overwrite_arrow_reader(
                        crate::arrow::batch_reader(
                            crate::arrow::arrow_schema_from_field(&schema()).unwrap(),
                            [rows_batch(&[1, 2])],
                        ),
                        &plain,
                    )
                    .unwrap();
                handle.reset_publications();
            }
            let options = plain.clone().with_commit_row_size(2);
            let source = counted_source(
                Arc::clone(&pulls),
                [
                    Ok(rows_batch(&[2, 3])),
                    Ok(rows_batch(&[99])),
                    Err(ArrowError::ComputeError("later source failure".into())),
                ],
            );
            let result = match intent {
                "overwrite" => handle.overwrite_arrow_reader(source, &options),
                "append" => handle.append_arrow_reader(source, &options),
                "merge" => {
                    handle.merge_arrow_reader(source, &options.clone().with_merge_by_names(["id"]))
                }
                _ => unreachable!(),
            };

            let message = result.unwrap_err().to_string();
            assert!(
                message.contains("later source failure"),
                "{intent}: {message}"
            );
            assert_eq!(handle.publications.load(Ordering::SeqCst), 1, "{intent}");
            assert_eq!(
                handle.pulls_when_published.lock().unwrap().as_slice(),
                [1],
                "{intent}: the second batch must not be pulled before commit one publishes"
            );
            assert_eq!(
                pulls.load(Ordering::SeqCst),
                3,
                "{intent}: the one-row second cadence is discarded when its next pull fails"
            );
            let expected_rows = match intent {
                "overwrite" => 2,
                "append" => 4,
                "merge" => 3,
                _ => unreachable!(),
            };
            assert_eq!(rows(&handle, &plain), expected_rows, "{intent}");
        }
    }

    #[test]
    fn a_second_publication_failure_keeps_the_first_commit_visible() {
        let pulls = Arc::new(AtomicUsize::new(0));
        let mut handle = PublicationProbe::new("second-publication-failure.arrows", pulls);
        handle.fail_on_publication(2);
        let options = handle
            .record_options()
            .unwrap()
            .with_field(schema())
            .with_commit_row_size(2);
        let source = crate::arrow::batch_reader(
            crate::arrow::arrow_schema_from_field(&schema()).unwrap(),
            [rows_batch(&[1, 2, 3, 4])],
        );

        let message = handle
            .overwrite_arrow_reader(source, &options)
            .unwrap_err()
            .to_string();

        assert!(message.contains("publication 2 refused"), "{message}");
        assert_eq!(handle.publications.load(Ordering::SeqCst), 2);
        assert_eq!(rows(&handle, &options), 2);
    }

    #[test]
    fn resumed_write_publishes_complete_cadences_and_abort_drops_only_the_remainder() {
        let pulls = Arc::new(AtomicUsize::new(0));
        let mut handle = PublicationProbe::new("resumed-write.arrows", pulls);
        let options = handle
            .record_options()
            .unwrap()
            .with_field(schema())
            .with_commit_row_size(3);
        let mut session = ArrowWriteSession::overwrite(&options).unwrap();

        assert!(
            session
                .push(
                    &mut handle,
                    crate::arrow::batch_reader(
                        crate::arrow::arrow_schema_from_field(&schema()).unwrap(),
                        [rows_batch(&[1, 2])],
                    ),
                )
                .unwrap()
        );
        assert_eq!(handle.publications.load(Ordering::SeqCst), 0);

        assert!(
            session
                .push(
                    &mut handle,
                    crate::arrow::batch_reader(
                        crate::arrow::arrow_schema_from_field(&schema()).unwrap(),
                        [rows_batch(&[3])],
                    ),
                )
                .unwrap()
        );
        assert_eq!(handle.publications.load(Ordering::SeqCst), 1);
        assert_eq!(rows(&handle, &options), 3);

        session
            .push(
                &mut handle,
                crate::arrow::batch_reader(
                    crate::arrow::arrow_schema_from_field(&schema()).unwrap(),
                    [rows_batch(&[4])],
                ),
            )
            .unwrap();
        session.abort();
        assert_eq!(handle.publications.load(Ordering::SeqCst), 1);
        assert_eq!(rows(&handle, &options), 3);
    }

    #[test]
    fn resumed_write_keeps_global_limits_and_stops_before_another_chunk_pull() {
        let pulls = Arc::new(AtomicUsize::new(0));
        let mut handle = PublicationProbe::new("resumed-limit.arrows", Arc::clone(&pulls));
        let options = handle
            .record_options()
            .unwrap()
            .with_field(schema())
            .with_commit_row_size(2)
            .with_max_row_size(3);
        let mut session = ArrowWriteSession::overwrite(&options).unwrap();

        assert!(
            session
                .push(
                    &mut handle,
                    counted_source(Arc::clone(&pulls), [Ok(rows_batch(&[1, 2]))]),
                )
                .unwrap()
        );
        let second = counted_source(
            Arc::clone(&pulls),
            [Ok(rows_batch(&[3, 4])), Ok(rows_batch(&[99]))],
        );
        assert!(!session.push(&mut handle, second).unwrap());
        session.finish(&mut handle).unwrap();

        assert_eq!(pulls.load(Ordering::SeqCst), 2);
        assert_eq!(handle.publications.load(Ordering::SeqCst), 2);
        assert_eq!(rows(&handle, &options), 3);
    }

    #[test]
    fn resumed_zero_limits_need_no_source_and_only_overwrite_publishes() {
        let pulls = Arc::new(AtomicUsize::new(0));
        let mut handle = PublicationProbe::new("resumed-zero.arrows", pulls);
        let base = handle
            .record_options()
            .unwrap()
            .with_field(schema())
            .with_commit_row_size(2)
            .with_max_row_size(0);
        handle.destination_touches.store(0, Ordering::SeqCst);

        let mut append = ArrowWriteSession::append(&base).unwrap();
        append.finish(&mut handle).unwrap();
        assert_eq!(handle.publications.load(Ordering::SeqCst), 0);
        assert_eq!(handle.destination_touches.load(Ordering::SeqCst), 0);

        let mut overwrite = ArrowWriteSession::overwrite(&base).unwrap();
        overwrite.finish(&mut handle).unwrap();
        assert_eq!(handle.publications.load(Ordering::SeqCst), 1);
        assert_eq!(rows(&handle, &base), 0);
        assert_eq!(handle.read_arrow_field(&base).unwrap(), schema());
    }

    #[test]
    fn resumed_sessions_keep_append_and_merge_intent_for_every_cadence() {
        let pulls = Arc::new(AtomicUsize::new(0));
        let mut handle = PublicationProbe::new("resumed-intents.arrows", pulls);
        let plain = handle.record_options().unwrap().with_field(schema());
        handle
            .overwrite_arrow_reader(
                crate::arrow::batch_reader(
                    crate::arrow::arrow_schema_from_field(&schema()).unwrap(),
                    [rows_batch(&[1, 2])],
                ),
                &plain,
            )
            .unwrap();

        handle.reset_publications();
        let append_options = plain.clone().with_commit_row_size(1);
        let mut append = ArrowWriteSession::append(&append_options).unwrap();
        append
            .push(
                &mut handle,
                crate::arrow::batch_reader(
                    crate::arrow::arrow_schema_from_field(&schema()).unwrap(),
                    [rows_batch(&[3, 4])],
                ),
            )
            .unwrap();
        append.finish(&mut handle).unwrap();
        assert_eq!(handle.publications.load(Ordering::SeqCst), 2);
        assert_eq!(rows(&handle, &plain), 4);

        handle.reset_publications();
        let merge_options = plain
            .clone()
            .with_commit_row_size(1)
            .with_merge_by_names(["id"]);
        let mut merge = ArrowWriteSession::merge(&merge_options).unwrap();
        merge
            .push(
                &mut handle,
                crate::arrow::batch_reader(
                    crate::arrow::arrow_schema_from_field(&schema()).unwrap(),
                    [rows_batch(&[2, 5])],
                ),
            )
            .unwrap();
        merge.finish(&mut handle).unwrap();
        assert_eq!(handle.publications.load(Ordering::SeqCst), 2);
        assert_eq!(rows(&handle, &plain), 5);
    }

    #[test]
    fn resumed_session_covers_large_cadence_multiple_commits_and_terminal_reuse() {
        let pulls = Arc::new(AtomicUsize::new(0));
        let mut large = PublicationProbe::new("resumed-large-cadence.arrows", Arc::clone(&pulls));
        let large_options = large
            .record_options()
            .unwrap()
            .with_field(schema())
            .with_commit_row_size(10);
        let mut session = ArrowWriteSession::overwrite(&large_options).unwrap();
        session
            .push(
                &mut large,
                crate::arrow::batch_reader(
                    crate::arrow::arrow_schema_from_field(&schema()).unwrap(),
                    [rows_batch(&[1, 2])],
                ),
            )
            .unwrap();
        assert_eq!(large.publications.load(Ordering::SeqCst), 0);
        session.finish(&mut large).unwrap();
        assert_eq!(large.publications.load(Ordering::SeqCst), 1);
        assert_eq!(rows(&large, &large_options), 2);
        let message = session
            .push(
                &mut large,
                crate::arrow::batch_reader(
                    crate::arrow::arrow_schema_from_field(&schema()).unwrap(),
                    [rows_batch(&[3])],
                ),
            )
            .unwrap_err()
            .to_string();
        assert!(message.contains("cannot be reused"), "{message}");

        let mut exact = PublicationProbe::new("resumed-multiple.arrows", pulls);
        let exact_options = exact
            .record_options()
            .unwrap()
            .with_field(schema())
            .with_commit_row_size(2);
        let mut exact_session = ArrowWriteSession::overwrite(&exact_options).unwrap();
        exact_session
            .push(
                &mut exact,
                crate::arrow::batch_reader(
                    crate::arrow::arrow_schema_from_field(&schema()).unwrap(),
                    [rows_batch(&[1, 2, 3, 4])],
                ),
            )
            .unwrap();
        exact_session.finish(&mut exact).unwrap();
        assert_eq!(exact.publications.load(Ordering::SeqCst), 2);
        assert_eq!(rows(&exact, &exact_options), 4);
    }

    #[test]
    fn resumed_session_fuses_on_schema_source_and_publication_failures() {
        let pulls = Arc::new(AtomicUsize::new(0));
        let mut mismatch = PublicationProbe::new("resumed-schema.arrows", Arc::clone(&pulls));
        let options = mismatch
            .record_options()
            .unwrap()
            .with_field(schema())
            .with_commit_row_size(2);
        let mut session = ArrowWriteSession::overwrite(&options).unwrap();
        session
            .push(
                &mut mismatch,
                crate::arrow::batch_reader(
                    crate::arrow::arrow_schema_from_field(&schema()).unwrap(),
                    [rows_batch(&[1])],
                ),
            )
            .unwrap();
        let other = DataType::from_fields([DataType::Utf8.required_field("id")])
            .unwrap()
            .required_field("row");
        let other_batch = RecordBatch::try_new(
            crate::arrow::arrow_schema_from_field(&other).unwrap(),
            vec![Arc::new(StringArray::from(vec!["2"]))],
        )
        .unwrap();
        let message = session
            .push(
                &mut mismatch,
                crate::arrow::batch_reader(other_batch.schema(), [other_batch]),
            )
            .unwrap_err()
            .to_string();
        assert!(message.contains("later chunk schema"), "{message}");
        assert_eq!(mismatch.publications.load(Ordering::SeqCst), 0);

        let mut source_failure =
            PublicationProbe::new("resumed-source-error.arrows", Arc::clone(&pulls));
        let source_options = source_failure
            .record_options()
            .unwrap()
            .with_field(schema())
            .with_commit_row_size(2);
        let mut source_session = ArrowWriteSession::overwrite(&source_options).unwrap();
        let error = source_session
            .push(
                &mut source_failure,
                counted_source(
                    Arc::clone(&pulls),
                    [
                        Ok(rows_batch(&[1, 2])),
                        Err(ArrowError::ComputeError("resumed source failed".into())),
                    ],
                ),
            )
            .unwrap_err();
        assert!(error.to_string().contains("resumed source failed"));
        assert_eq!(source_failure.publications.load(Ordering::SeqCst), 1);
        assert_eq!(rows(&source_failure, &source_options), 2);

        let mut publication = PublicationProbe::new("resumed-publication-error.arrows", pulls);
        publication.fail_on_publication(2);
        let publication_options = publication
            .record_options()
            .unwrap()
            .with_field(schema())
            .with_commit_row_size(1);
        let mut publication_session = ArrowWriteSession::overwrite(&publication_options).unwrap();
        let error = publication_session
            .push(
                &mut publication,
                crate::arrow::batch_reader(
                    crate::arrow::arrow_schema_from_field(&schema()).unwrap(),
                    [rows_batch(&[1, 2])],
                ),
            )
            .unwrap_err();
        assert!(error.to_string().contains("publication 2 refused"));
        assert_eq!(publication.publications.load(Ordering::SeqCst), 2);
        assert_eq!(rows(&publication, &publication_options), 1);
    }

    #[test]
    fn resumed_leaf_keeps_the_target_captured_before_an_external_replacement() {
        let pulls = Arc::new(AtomicUsize::new(0));
        let mut handle = PublicationProbe::new("resumed-stable-target.arrows", pulls);
        let options = handle
            .record_options()
            .unwrap()
            .with_field(schema())
            .with_commit_row_size(1);
        let mut session = ArrowWriteSession::overwrite(&options).unwrap();
        session
            .push(
                &mut handle,
                crate::arrow::batch_reader(
                    crate::arrow::arrow_schema_from_field(&schema()).unwrap(),
                    [rows_batch(&[1])],
                ),
            )
            .unwrap();

        handle.clear().unwrap();
        let loose = DataType::from_fields([DataType::Utf8.required_field("id")])
            .unwrap()
            .required_field("other");
        let loose_batch = RecordBatch::try_new(
            crate::arrow::arrow_schema_from_field(&loose).unwrap(),
            vec![Arc::new(StringArray::from(vec!["9"]))],
        )
        .unwrap();
        let external = handle.record_options().unwrap();
        handle
            .overwrite_arrow_reader(
                crate::arrow::batch_reader(loose_batch.schema(), [loose_batch]),
                &external,
            )
            .unwrap();
        let replaced = handle.read_arrow_field(&external).unwrap();
        assert_eq!(replaced.dtype(), loose.dtype());
        assert_ne!(replaced, schema());

        session
            .push(
                &mut handle,
                crate::arrow::batch_reader(
                    crate::arrow::arrow_schema_from_field(&schema()).unwrap(),
                    [rows_batch(&[2])],
                ),
            )
            .unwrap();
        session.finish(&mut handle).unwrap();

        assert_eq!(handle.read_arrow_field(&options).unwrap(), schema());
        assert_eq!(rows(&handle, &options), 2);
    }

    #[test]
    fn bounded_empty_intents_publish_only_overwrite() {
        let pulls = Arc::new(AtomicUsize::new(0));
        let mut handle = PublicationProbe::new("bounded-empty.arrows", pulls);
        let plain = handle.record_options().unwrap().with_field(schema());
        handle.overwrite_arrow_reader(reader(), &plain).unwrap();
        handle.reset_publications();
        let bounded = plain.clone().with_commit_row_size(2);
        let empty = || {
            crate::arrow::batch_reader(
                crate::arrow::arrow_schema_from_field(&schema()).unwrap(),
                [],
            )
        };

        handle.append_arrow_reader(empty(), &bounded).unwrap();
        handle
            .merge_arrow_reader(empty(), &bounded.clone().with_merge_by_names(["id"]))
            .unwrap();
        assert_eq!(handle.publications.load(Ordering::SeqCst), 0);
        assert_eq!(rows(&handle, &plain), 2);

        handle.overwrite_arrow_reader(empty(), &bounded).unwrap();
        assert_eq!(handle.publications.load(Ordering::SeqCst), 1);
        assert_eq!(rows(&handle, &plain), 0);
        assert_eq!(handle.read_arrow_field(&plain).unwrap(), schema());
    }

    #[cfg(feature = "parquet")]
    #[test]
    fn the_three_methods_behave_the_same_way_on_parquet() {
        let mut handle = handle("three.parquet");
        let options = handle.record_options().unwrap().with_field(schema());

        handle.append_arrow_reader(reader(), &options).unwrap();
        handle.overwrite_arrow_reader(reader(), &options).unwrap();
        handle.append_arrow_reader(reader(), &options).unwrap();

        assert_eq!(rows(&handle, &options), 4);
    }

    #[test]
    fn record_batch_adapters_route_to_each_explicit_reader_primitive() {
        let mut handle = handle("record-batch-adapters.arrows");
        let options = handle.record_options().unwrap().with_field(schema());

        handle.overwrite_arrow_batch(batch(), &options).unwrap();
        handle.append_arrow_batch(batch(), &options).unwrap();
        assert_eq!(rows(&handle, &options), 4);

        let merging = options.clone().with_merge_by_names(["id"]);
        handle.merge_arrow_batch(batch(), &merging).unwrap();
        // Both stored copies of each key update in place; merge does not turn
        // either incoming row into a third copy.
        assert_eq!(rows(&handle, &options), 4);
    }

    #[derive(Clone)]
    struct NativeRow {
        id: i64,
        symbol: Option<&'static str>,
    }

    impl From<NativeRow> for Scalar {
        fn from(row: NativeRow) -> Self {
            Scalar::from_sequence([
                Scalar::from(row.id),
                row.symbol.map_or(Scalar::Null, Scalar::from),
            ])
        }
    }

    #[test]
    fn generic_write_entry_points_compose_the_three_typed_shapes() {
        let mut handle = handle("generic-write-mode.arrows");
        let options = handle
            .record_options()
            .unwrap()
            .with_field(schema())
            .with_batch_size(1);

        handle
            .write_arrow_reader(reader(), IOMode::Overwrite, &options)
            .unwrap();
        handle
            .write_arrow_batch(rows_batch(&[3]), IOMode::Append, &options)
            .unwrap();
        handle
            .write_records(
                [
                    NativeRow {
                        id: 2,
                        symbol: Some("updated"),
                    },
                    NativeRow {
                        id: 4,
                        symbol: Some("AMD"),
                    },
                ],
                IOMode::Merge,
                &options.clone().with_merge_by_names(["id"]),
            )
            .unwrap();

        assert_eq!(rows(&handle, &options), 4);
    }

    #[test]
    fn generic_write_entry_points_preserve_commit_cadence() {
        let pulls = Arc::new(AtomicUsize::new(0));
        let mut reader_handle =
            PublicationProbe::new("generic-reader-commits.arrows", Arc::clone(&pulls));
        let options = reader_handle
            .record_options()
            .unwrap()
            .with_field(schema())
            .with_commit_row_size(1);
        reader_handle
            .write_arrow_reader(reader(), IOMode::Overwrite, &options)
            .unwrap();
        assert_eq!(reader_handle.publications.load(Ordering::SeqCst), 2);

        let mut batch_handle =
            PublicationProbe::new("generic-batch-commits.arrows", Arc::clone(&pulls));
        batch_handle
            .write_arrow_batch(batch(), IOMode::Overwrite, &options)
            .unwrap();
        assert_eq!(batch_handle.publications.load(Ordering::SeqCst), 2);

        let mut record_handle = PublicationProbe::new("generic-row-commits.arrows", pulls);
        record_handle
            .write_records(
                [
                    NativeRow {
                        id: 1,
                        symbol: Some("AAPL"),
                    },
                    NativeRow {
                        id: 2,
                        symbol: Some("MSFT"),
                    },
                ],
                IOMode::Overwrite,
                &options,
            )
            .unwrap();
        assert_eq!(record_handle.publications.load(Ordering::SeqCst), 2);
    }

    /// A media may preserve a same-shape optimization while still converging
    /// on the reader primitives. The generic entry points must select that
    /// authoritative adapter rather than rebuilding its input themselves.
    struct TypedDispatchProbe {
        handle: Buffer,
        reader_calls: [usize; 3],
        batch_calls: [usize; 3],
        record_calls: [usize; 3],
    }

    impl TypedDispatchProbe {
        fn new() -> Self {
            Self {
                handle: handle("typed-dispatch-probe.arrows"),
                reader_calls: [0; 3],
                batch_calls: [0; 3],
                record_calls: [0; 3],
            }
        }
    }

    impl IOMedia for TypedDispatchProbe {
        fn as_io_base(&self) -> &dyn IOBase {
            self
        }

        fn as_io_base_mut(&mut self) -> &mut dyn IOBase {
            self
        }

        fn overwrite_arrow_reader(
            &mut self,
            _batches: BatchReader,
            _options: &RecordOptions,
        ) -> crate::Result<()> {
            self.reader_calls[0] += 1;
            Ok(())
        }

        fn append_arrow_reader(
            &mut self,
            _batches: BatchReader,
            _options: &RecordOptions,
        ) -> crate::Result<()> {
            self.reader_calls[1] += 1;
            Ok(())
        }

        fn merge_arrow_reader(
            &mut self,
            _batches: BatchReader,
            _options: &RecordOptions,
        ) -> crate::Result<()> {
            self.reader_calls[2] += 1;
            Ok(())
        }

        fn overwrite_arrow_batch(
            &mut self,
            _batch: RecordBatch,
            _options: &RecordOptions,
        ) -> crate::Result<()> {
            self.batch_calls[0] += 1;
            Ok(())
        }

        fn append_arrow_batch(
            &mut self,
            _batch: RecordBatch,
            _options: &RecordOptions,
        ) -> crate::Result<()> {
            self.batch_calls[1] += 1;
            Ok(())
        }

        fn merge_arrow_batch(
            &mut self,
            _batch: RecordBatch,
            _options: &RecordOptions,
        ) -> crate::Result<()> {
            self.batch_calls[2] += 1;
            Ok(())
        }

        fn overwrite_records<I, R>(
            &mut self,
            _records: I,
            _options: &RecordOptions,
        ) -> crate::Result<()>
        where
            Self: Sized,
            I: IntoIterator<Item = R>,
            I::IntoIter: Send + 'static,
            R: TryInto<Scalar>,
            R::Error: Into<Error>,
        {
            self.record_calls[0] += 1;
            Ok(())
        }

        fn append_records<I, R>(
            &mut self,
            _records: I,
            _options: &RecordOptions,
        ) -> crate::Result<()>
        where
            Self: Sized,
            I: IntoIterator<Item = R>,
            I::IntoIter: Send + 'static,
            R: TryInto<Scalar>,
            R::Error: Into<Error>,
        {
            self.record_calls[1] += 1;
            Ok(())
        }

        fn merge_records<I, R>(
            &mut self,
            _records: I,
            _options: &RecordOptions,
        ) -> crate::Result<()>
        where
            Self: Sized,
            I: IntoIterator<Item = R>,
            I::IntoIter: Send + 'static,
            R: TryInto<Scalar>,
            R::Error: Into<Error>,
        {
            self.record_calls[2] += 1;
            Ok(())
        }
    }

    impl IOBase for TypedDispatchProbe {
        crate::delegate_iobase!(handle);
    }

    #[test]
    fn generic_writes_select_the_same_shape_for_every_mode() {
        let mut probe = TypedDispatchProbe::new();
        let plain = probe.record_options().unwrap().with_field(schema());

        for mode in IOMode::WRITE {
            let options = if mode == IOMode::Merge {
                plain.clone().with_merge_by_names(["id"])
            } else {
                plain.clone()
            };
            probe.write_arrow_reader(reader(), mode, &options).unwrap();
            probe.write_arrow_batch(batch(), mode, &options).unwrap();
            probe
                .write_records(std::iter::empty::<NativeRow>(), mode, &options)
                .unwrap();
        }

        assert_eq!(probe.reader_calls, [1, 1, 1]);
        assert_eq!(probe.batch_calls, [1, 1, 1]);
        assert_eq!(probe.record_calls, [1, 1, 1]);
    }

    #[test]
    fn generic_arrow_writes_remain_object_safe() {
        let mut probe = TypedDispatchProbe::new();
        let options = probe.record_options().unwrap().with_field(schema());
        let media: &mut dyn IOMedia = &mut probe;

        media
            .write_arrow_reader(reader(), IOMode::Overwrite, &options)
            .unwrap();
        media
            .write_arrow_batch(batch(), IOMode::Append, &options)
            .unwrap();

        assert_eq!(probe.reader_calls, [1, 0, 0]);
        assert_eq!(probe.batch_calls, [0, 1, 0]);
    }

    #[test]
    fn generic_writes_validate_mode_before_touching_input() {
        let mut handle = handle("generic-write-mode-validation.arrows");
        let options = handle.record_options().unwrap().with_field(schema());

        // The required mode is validated before a one-shot source is pulled;
        // a match key never silently turns overwrite into merge.
        let pulls = Arc::new(AtomicUsize::new(0));
        let source = counted_source(Arc::clone(&pulls), [Ok(rows_batch(&[5]))]);
        let error = handle
            .write_arrow_reader(
                source,
                IOMode::Overwrite,
                &options.clone().with_merge_by_names(["id"]),
            )
            .unwrap_err();
        assert!(error.to_string().contains("write mode overwrite"));
        assert_eq!(pulls.load(Ordering::SeqCst), 0);

        // A held batch is already materialized, but invalid intent still does
        // not reach the destination or silently select merge.
        let before = handle.as_slice().to_vec();
        let error = handle
            .write_arrow_batch(
                rows_batch(&[6]),
                IOMode::Overwrite,
                &options.clone().with_merge_by_names(["id"]),
            )
            .unwrap_err();
        assert!(error.to_string().contains("write mode overwrite"));
        assert_eq!(handle.as_slice(), before.as_slice());

        struct CountedIntoRows(Arc<AtomicUsize>);

        impl IntoIterator for CountedIntoRows {
            type Item = NativeRow;
            type IntoIter = std::iter::Empty<NativeRow>;

            fn into_iter(self) -> Self::IntoIter {
                self.0.fetch_add(1, Ordering::SeqCst);
                std::iter::empty()
            }
        }

        // Mode validation also wins over the missing-field error and happens
        // before even constructing a native row iterator.
        let into_iters = Arc::new(AtomicUsize::new(0));
        let untyped = handle.record_options().unwrap().with_merge_by_names(["id"]);
        let error = handle
            .write_records(
                CountedIntoRows(Arc::clone(&into_iters)),
                IOMode::Overwrite,
                &untyped,
            )
            .unwrap_err();
        assert!(error.to_string().contains("write mode overwrite"));
        assert_eq!(into_iters.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn native_struct_row_adapters_route_all_three_intents() {
        let mut handle = handle("native-row-adapters.arrows");
        let options = handle
            .record_options()
            .unwrap()
            .with_field(schema())
            .with_batch_size(1);

        handle
            .overwrite_records(
                [
                    NativeRow {
                        id: 1,
                        symbol: Some("AAPL"),
                    },
                    NativeRow {
                        id: 2,
                        symbol: None,
                    },
                ],
                &options,
            )
            .unwrap();
        handle
            .append_records(
                [NativeRow {
                    id: 3,
                    symbol: Some("MSFT"),
                }],
                &options,
            )
            .unwrap();
        handle
            .merge_records(
                [
                    NativeRow {
                        id: 2,
                        symbol: Some("updated"),
                    },
                    NativeRow {
                        id: 4,
                        symbol: Some("AMD"),
                    },
                ],
                &options.clone().with_merge_by_names(["id"]),
            )
            .unwrap();

        assert_eq!(rows(&handle, &options), 4);
        assert_eq!(handle.read_arrow_field(&options).unwrap(), schema());
    }

    #[test]
    fn native_row_methods_require_a_field_before_pulling() {
        use std::sync::atomic::{AtomicUsize, Ordering};

        struct CountedRows(Arc<AtomicUsize>);

        impl Iterator for CountedRows {
            type Item = NativeRow;

            fn next(&mut self) -> Option<Self::Item> {
                self.0.fetch_add(1, Ordering::SeqCst);
                Some(NativeRow {
                    id: 1,
                    symbol: None,
                })
            }
        }

        for intent in ["overwrite", "append", "merge"] {
            let pulls = Arc::new(AtomicUsize::new(0));
            let mut handle = handle(&format!("native-row-no-field-{intent}.arrows"));
            let options = handle.record_options().unwrap();
            let result = match intent {
                "overwrite" => handle.overwrite_records(CountedRows(Arc::clone(&pulls)), &options),
                "append" => handle.append_records(CountedRows(Arc::clone(&pulls)), &options),
                "merge" => handle.merge_records(
                    CountedRows(Arc::clone(&pulls)),
                    &options.with_merge_by_names(["id"]),
                ),
                _ => unreachable!(),
            };
            let message = result.unwrap_err().to_string();
            assert!(message.contains("with_field"), "{intent}: {message}");
            assert_eq!(pulls.load(Ordering::SeqCst), 0, "{intent}");
            assert!(handle.is_empty(), "{intent}");
        }
    }

    #[test]
    fn native_row_methods_validate_intent_before_building_or_pulling_the_iterator() {
        use std::sync::atomic::{AtomicUsize, Ordering};

        for intent in ["overwrite", "append", "merge"] {
            let pulls = Arc::new(AtomicUsize::new(0));
            let counted = Arc::clone(&pulls);
            let records = std::iter::from_fn(move || {
                counted.fetch_add(1, Ordering::SeqCst);
                Some(NativeRow {
                    id: 1,
                    symbol: None,
                })
            });
            let mut handle = handle(&format!("native-row-invalid-intent-{intent}.arrows"));
            let plain = handle.record_options().unwrap().with_field(schema());
            let result = match intent {
                "overwrite" => {
                    handle.overwrite_records(records, &plain.clone().with_merge_by_names(["id"]))
                }
                "append" => {
                    handle.append_records(records, &plain.clone().with_merge_by_names(["id"]))
                }
                "merge" => handle.merge_records(records, &plain),
                _ => unreachable!(),
            };

            let message = result.unwrap_err().to_string();
            assert!(message.contains("merge_by_names"), "{intent}: {message}");
            assert_eq!(pulls.load(Ordering::SeqCst), 0, "{intent}");
            assert!(handle.is_empty(), "{intent}");
        }
    }

    struct FallibleRow(std::result::Result<Scalar, Error>);

    impl TryFrom<FallibleRow> for Scalar {
        type Error = Error;

        fn try_from(row: FallibleRow) -> std::result::Result<Self, Self::Error> {
            row.0
        }
    }

    struct CountedFallibleRow {
        value: std::result::Result<Scalar, Error>,
        conversions: Arc<AtomicUsize>,
    }

    impl TryFrom<CountedFallibleRow> for Scalar {
        type Error = Error;

        fn try_from(row: CountedFallibleRow) -> std::result::Result<Self, Self::Error> {
            row.conversions.fetch_add(1, Ordering::SeqCst);
            row.value
        }
    }

    #[test]
    fn native_row_conversion_failure_is_typed_and_does_not_publish() {
        let mut handle = handle("native-row-failure.arrows");
        let options = handle
            .record_options()
            .unwrap()
            .with_field(schema())
            .with_batch_size(1);
        handle
            .overwrite_records(
                [NativeRow {
                    id: 9,
                    symbol: Some("kept"),
                }],
                &options,
            )
            .unwrap();
        let before = handle.as_slice().to_vec();

        let error = handle
            .append_records(
                [
                    FallibleRow(Ok(Scalar::from_sequence([
                        Scalar::from(10_i64),
                        Scalar::from("not-published"),
                    ]))),
                    FallibleRow(Err(Error::InvalidRecord {
                        path: "$.row".into(),
                        reason: "conversion refused".into(),
                    })),
                ],
                &options,
            )
            .unwrap_err();
        assert!(matches!(error, Error::InvalidRecord { .. }));
        assert_eq!(handle.as_slice(), before.as_slice());
    }

    #[test]
    fn native_row_conversion_stops_at_each_commit_for_all_intents() {
        for intent in ["overwrite", "append", "merge"] {
            let conversions = Arc::new(AtomicUsize::new(0));
            let mut handle = PublicationProbe::new(
                &format!("native-partial-{intent}.arrows"),
                Arc::clone(&conversions),
            );
            let plain = handle.record_options().unwrap().with_field(schema());
            if intent != "overwrite" {
                handle.overwrite_arrow_reader(reader(), &plain).unwrap();
                handle.reset_publications();
            }
            let records = [
                CountedFallibleRow {
                    value: Ok(Scalar::from_sequence([
                        Scalar::from(3_i64),
                        Scalar::from("committed"),
                    ])),
                    conversions: Arc::clone(&conversions),
                },
                CountedFallibleRow {
                    value: Err(Error::InvalidRecord {
                        path: "$.row[1]".into(),
                        reason: "later native conversion failure".into(),
                    }),
                    conversions: Arc::clone(&conversions),
                },
            ];
            let committed = plain.clone().with_commit_row_size(1);
            let result = match intent {
                "overwrite" => handle.overwrite_records(records, &committed),
                "append" => handle.append_records(records, &committed),
                "merge" => {
                    handle.merge_records(records, &committed.clone().with_merge_by_names(["id"]))
                }
                _ => unreachable!(),
            };

            let error = result.unwrap_err();
            assert!(matches!(error, Error::InvalidRecord { .. }), "{intent}");
            assert_eq!(handle.publications.load(Ordering::SeqCst), 1, "{intent}");
            assert_eq!(
                handle.pulls_when_published.lock().unwrap().as_slice(),
                [1],
                "{intent}: row two must not convert before row one publishes"
            );
            assert_eq!(conversions.load(Ordering::SeqCst), 2, "{intent}");
            assert_eq!(
                rows(&handle, &plain),
                if intent == "overwrite" { 1 } else { 3 },
                "{intent}"
            );
        }
    }

    #[test]
    fn native_rows_align_a_non_divisible_batch_before_the_next_conversion() {
        let conversions = Arc::new(AtomicUsize::new(0));
        let mut handle =
            PublicationProbe::new("native-non-divisible.arrows", Arc::clone(&conversions));
        let options = handle
            .record_options()
            .unwrap()
            .with_field(schema())
            .with_batch_size(2)
            .with_commit_row_size(3);
        let mut records = Vec::new();
        for id in 1..=3_i64 {
            records.push(CountedFallibleRow {
                value: Ok(Scalar::from_sequence([
                    Scalar::from(id),
                    Scalar::from("committed"),
                ])),
                conversions: Arc::clone(&conversions),
            });
        }
        records.push(CountedFallibleRow {
            value: Err(Error::InvalidRecord {
                path: "$.row[3]".into(),
                reason: "conversion after a non-divisible cadence".into(),
            }),
            conversions: Arc::clone(&conversions),
        });

        let error = handle.overwrite_records(records, &options).unwrap_err();
        assert!(matches!(error, Error::InvalidRecord { .. }));
        assert_eq!(handle.publications.load(Ordering::SeqCst), 1);
        assert_eq!(
            handle.pulls_when_published.lock().unwrap().as_slice(),
            [3],
            "row four must not convert before the three-row cadence publishes"
        );
        assert_eq!(conversions.load(Ordering::SeqCst), 4);
        assert_eq!(rows(&handle, &options), 3);
    }

    #[test]
    fn native_rows_stop_at_the_global_row_limit_without_one_extra_pull() {
        let pulls = Arc::new(AtomicUsize::new(0));
        let counted = Arc::clone(&pulls);
        let records = std::iter::from_fn(move || {
            let id = counted.fetch_add(1, Ordering::SeqCst) as i64;
            Some(NativeRow {
                id,
                symbol: Some("bounded"),
            })
        });
        let mut handle = handle("native-global-row-limit.arrows");
        let options = handle
            .record_options()
            .unwrap()
            .with_field(schema())
            .with_batch_size(2)
            .with_max_row_size(3);

        handle.overwrite_records(records, &options).unwrap();

        assert_eq!(pulls.load(Ordering::SeqCst), 3);
        assert_eq!(rows(&handle, &options), 3);
    }

    #[test]
    fn empty_native_row_intents_keep_overwrite_schema_and_make_append_merge_no_ops() {
        let mut missing = handle("empty-native-row-append.arrows");
        let options = missing.record_options().unwrap().with_field(schema());
        missing
            .append_records(std::iter::empty::<NativeRow>(), &options)
            .unwrap();
        assert!(missing.is_empty());

        let mut handle = handle("empty-native-row-overwrite.arrows");
        handle
            .overwrite_records(std::iter::empty::<NativeRow>(), &options)
            .unwrap();
        assert_eq!(rows(&handle, &options), 0);
        assert_eq!(handle.read_arrow_field(&options).unwrap(), schema());
        let before = handle.as_slice().to_vec();

        handle
            .append_records(
                std::iter::empty::<NativeRow>(),
                &options.clone().with_select_by_names(["absent"]),
            )
            .unwrap();
        handle
            .merge_records(
                std::iter::empty::<NativeRow>(),
                &options
                    .clone()
                    .with_merge_by_names(["id"])
                    .with_select_by_names(["absent"]),
            )
            .unwrap();
        assert_eq!(handle.as_slice(), before.as_slice());
    }

    #[test]
    fn an_empty_record_batch_overwrite_keeps_its_field_and_no_rows() {
        let mut handle = handle("empty-record-batch.arrows");
        let options = handle.record_options().unwrap();
        let empty =
            RecordBatch::new_empty(crate::arrow::arrow_schema_from_field(&schema()).unwrap());

        handle.overwrite_arrow_batch(empty, &options).unwrap();

        assert_eq!(rows(&handle, &options), 0);
        assert_eq!(handle.read_arrow_field(&options).unwrap(), schema());
    }

    #[test]
    fn empty_append_and_merge_are_byte_for_byte_no_ops() {
        let mut missing = handle("empty-no-op.arrows");
        let options = missing.record_options().unwrap().with_field(schema());
        missing
            .append_arrow_reader(
                crate::arrow::batch_reader(
                    crate::arrow::arrow_schema_from_field(&schema()).unwrap(),
                    [],
                ),
                &options.clone().with_select_by_names(["absent"]),
            )
            .unwrap();
        assert!(missing.is_empty(), "an empty append must not create bytes");

        missing.overwrite_arrow_reader(reader(), &options).unwrap();
        let before = missing.as_slice().to_vec();
        let zero =
            RecordBatch::new_empty(crate::arrow::arrow_schema_from_field(&schema()).unwrap());
        missing
            .merge_arrow_reader(
                crate::arrow::batch_reader(zero.schema(), [zero]),
                &options
                    .clone()
                    .with_merge_by_names(["id"])
                    .with_select_by_names(["absent"]),
            )
            .unwrap();
        assert_eq!(missing.as_slice(), before.as_slice());
    }

    #[test]
    fn appending_casts_incoming_batches_to_the_target_shape() {
        let mut handle = handle("cast-append.arrows");
        let options = handle.record_options().unwrap().with_field(schema());
        handle.overwrite_arrow_reader(reader(), &options).unwrap();

        // The incoming batch merely fits: `id` is narrower and the columns are
        // the other way round.
        let loose = DataType::from_fields([
            DataType::Utf8.nullable_field("symbol"),
            DataType::Int32.required_field("id"),
        ])
        .unwrap()
        .required_field("row");
        let incoming = RecordBatch::try_new(
            crate::arrow::arrow_schema_from_field(&loose).unwrap(),
            vec![
                Arc::new(StringArray::from(vec![Some("MSFT")])),
                Arc::new(arrow_array::Int32Array::from(vec![3])),
            ],
        )
        .unwrap();

        handle
            .append_arrow_reader(
                crate::arrow::batch_reader(incoming.schema(), [incoming]),
                &options,
            )
            .unwrap();

        let batches = handle
            .read_arrow_reader(&options)
            .unwrap()
            .map(std::result::Result::unwrap)
            .collect::<Vec<_>>();
        assert_eq!(batches.len(), 2);
        assert_eq!(batches[1].schema(), batches[0].schema());
        assert_eq!(batches[1].num_rows(), 1);
    }

    #[test]
    fn a_cast_that_cannot_be_planned_leaves_the_resource_alone() {
        let mut handle = handle("failed-append.arrows");
        let options = handle.record_options().unwrap().with_field(schema());
        handle.overwrite_arrow_reader(reader(), &options).unwrap();
        let before = handle.as_slice().to_vec();

        // Text that is not a number cannot become the declared Int64, and this
        // write is strict, so the append fails while the batches are being
        // encoded - before anything is published.
        let hostile = DataType::from_fields([DataType::Utf8.required_field("id")])
            .unwrap()
            .required_field("row");
        let incoming = RecordBatch::try_new(
            crate::arrow::arrow_schema_from_field(&hostile).unwrap(),
            vec![Arc::new(StringArray::from(vec!["not a number"]))],
        )
        .unwrap();

        let message = handle
            .append_arrow_reader(
                crate::arrow::batch_reader(incoming.schema(), [incoming]),
                &options,
            )
            .unwrap_err()
            .to_string();

        // The core failure is reported as itself rather than as the Arrow
        // envelope it had to travel through the reader inside.
        assert!(!message.contains("External error"), "{message}");
        assert_eq!(handle.as_slice(), before.as_slice());
    }

    #[test]
    fn a_row_limit_bounds_a_read_at_below_and_above_the_stored_count() {
        let mut handle = handle("row-limited.arrows");
        let options = handle.record_options().unwrap().with_field(schema());
        handle.overwrite_arrow_reader(reader(), &options).unwrap();

        // The bound is exact: one below slices, the count itself keeps
        // everything, and one above changes nothing.
        assert_eq!(rows(&handle, &options.clone().with_max_row_size(1)), 1);
        assert_eq!(rows(&handle, &options.clone().with_max_row_size(2)), 2);
        assert_eq!(rows(&handle, &options.clone().with_max_row_size(3)), 2);
    }

    #[test]
    fn overwrite_refuses_a_match_key_and_a_limited_merge_names_both_settings() {
        let mut handle = handle("limited-merge.arrows");
        let keyed = handle
            .record_options()
            .unwrap()
            .with_field(schema())
            .with_merge_by_names(["id"]);
        let before = handle.as_slice().to_vec();

        // The operation carries intent: overwrite never silently becomes a
        // merge because its options happen to carry keys.
        let message = handle
            .overwrite_arrow_reader(reader(), &keyed)
            .unwrap_err()
            .to_string();
        assert!(message.contains("write mode overwrite"), "{message}");
        assert!(message.contains("merge_by_names"), "{message}");
        assert_eq!(handle.as_slice(), before.as_slice());

        // A truncated merge would update the matched keys it kept and
        // silently drop the rest, so the combination is refused by name
        // before a single row moves.
        let limited = keyed.with_max_row_size(1);
        let message = handle
            .merge_arrow_reader(reader(), &limited)
            .unwrap_err()
            .to_string();
        assert!(message.contains("max_row_size = 1"), "{message}");
        assert!(message.contains("merge_by_names [\"id\"]"), "{message}");
        assert_eq!(handle.as_slice(), before.as_slice());
    }

    #[test]
    fn merge_refuses_an_empty_match_key_before_pulling_the_reader() {
        use std::sync::atomic::{AtomicUsize, Ordering};

        struct Counting {
            schema: arrow_schema::SchemaRef,
            pulls: Arc<AtomicUsize>,
        }

        impl Iterator for Counting {
            type Item = std::result::Result<RecordBatch, arrow_schema::ArrowError>;

            fn next(&mut self) -> Option<Self::Item> {
                self.pulls.fetch_add(1, Ordering::SeqCst);
                Some(Ok(batch()))
            }
        }

        impl RecordBatchReader for Counting {
            fn schema(&self) -> arrow_schema::SchemaRef {
                Arc::clone(&self.schema)
            }
        }

        let pulls = Arc::new(AtomicUsize::new(0));
        let reader: BatchReader = Box::new(Counting {
            schema: crate::arrow::arrow_schema_from_field(&schema()).unwrap(),
            pulls: Arc::clone(&pulls),
        });
        let mut handle = handle("missing-merge-key.arrows");
        let options = handle.record_options().unwrap().with_field(schema());
        let message = handle
            .merge_arrow_reader(reader, &options)
            .unwrap_err()
            .to_string();

        assert!(message.contains("requires at least one"), "{message}");
        assert!(message.contains("merge_by_names"), "{message}");
        assert_eq!(pulls.load(Ordering::SeqCst), 0);
        assert!(handle.is_empty());
    }

    /// A declared schema is what a read selects *and* casts to: the columns it
    /// names become the encoding's own projection, and what comes back is the
    /// shape it declares rather than the shape the resource stores.
    mod pushdown {
        use super::{Buffer, DataType, Field, IORecordOptions, RecordBatchReader, handle};

        use std::sync::Arc;

        use arrow_array::{Float64Array, Int64Array, RecordBatch, StringArray};

        use crate::io::IOMedia;

        /// Four columns, so a two-column read is a genuine subset.
        fn wide() -> Field {
            DataType::from_fields([
                DataType::Int64.required_field("id"),
                DataType::Utf8.nullable_field("symbol"),
                DataType::Float64.required_field("price"),
                DataType::Utf8.nullable_field("venue"),
            ])
            .unwrap()
            .required_field("row")
        }

        /// The two columns a caller actually wants.
        fn narrow() -> Field {
            DataType::from_fields([
                DataType::Int64.required_field("id"),
                DataType::Float64.required_field("price"),
            ])
            .unwrap()
            .required_field("row")
        }

        fn stored(name: &str) -> Buffer {
            let mut handle = handle(name);
            let options = handle.record_options().unwrap();
            let batch = RecordBatch::try_new(
                crate::arrow::arrow_schema_from_field(&wide()).unwrap(),
                vec![
                    Arc::new(Int64Array::from(vec![1, 2])),
                    Arc::new(StringArray::from(vec![Some("AAPL"), None])),
                    Arc::new(Float64Array::from(vec![1.5, 2.5])),
                    Arc::new(StringArray::from(vec![Some("XNAS"), None])),
                ],
            )
            .unwrap();
            handle
                .overwrite_arrow_reader(
                    crate::arrow::batch_reader(batch.schema(), [batch]),
                    &options,
                )
                .unwrap();
            handle
        }

        #[test]
        fn a_subset_schema_narrows_the_batches_every_encoding_yields() {
            let mut names = vec!["pushdown.arrows"];
            if cfg!(feature = "parquet") {
                names.push("pushdown.parquet");
            }

            for name in names {
                let handle = stored(name);
                let plain = handle.record_options().unwrap();

                // The resource still holds four columns.
                assert_eq!(
                    handle.read_arrow_field(&plain).unwrap().field_len(),
                    4,
                    "{name}"
                );

                let options = plain.with_field(narrow());
                let reader = handle
                    .read_arrow_reader(&options)
                    .unwrap_or_else(|error| panic!("{name}: {error}"));
                // The schema is narrowed before a single batch is decoded.
                assert_eq!(reader.schema().fields().len(), 2, "{name}");
                let batches = reader.map(std::result::Result::unwrap).collect::<Vec<_>>();
                assert_eq!(batches.len(), 1, "{name}");
                assert_eq!(batches[0].num_columns(), 2, "{name}");
                assert_eq!(batches[0].num_rows(), 2, "{name}");
                assert_eq!(batches[0].schema().field(0).name(), "id", "{name}");
                assert_eq!(batches[0].schema().field(1).name(), "price", "{name}");
            }
        }

        #[test]
        fn the_projection_only_drops_columns_and_the_cast_does_the_rest() {
            let handle = stored("unprojected.arrows");
            let plain = handle.record_options().unwrap();

            // Every stored column: there is nothing to skip.
            let all = handle
                .read_arrow_reader(&plain.clone().with_field(wide()))
                .unwrap()
                .schema();
            assert_eq!(all.fields().len(), 4);

            // A column the resource does not hold cannot be projected out of
            // it, so the encoding reads everything and the cast supplies it.
            let invented = DataType::from_fields([
                DataType::Int64.required_field("id"),
                DataType::Utf8.nullable_field("nowhere"),
            ])
            .unwrap()
            .required_field("row");
            let batches = handle
                .read_arrow_reader(&plain.with_field(invented))
                .unwrap()
                .map(std::result::Result::unwrap)
                .collect::<Vec<_>>();
            assert_eq!(batches[0].num_columns(), 2);
            assert_eq!(batches[0].schema().field(1).name(), "nowhere");
            assert_eq!(batches[0].column(1).null_count(), 2);
        }

        #[test]
        fn a_declared_schema_reorders_what_the_resource_stores() {
            let handle = stored("reordered.arrows");
            let reversed = DataType::from_fields([
                DataType::Float64.required_field("price"),
                DataType::Int64.required_field("id"),
            ])
            .unwrap()
            .required_field("row");
            let options = handle.record_options().unwrap().with_field(reversed);

            let batches = handle
                .read_arrow_reader(&options)
                .unwrap()
                .map(std::result::Result::unwrap)
                .collect::<Vec<_>>();
            assert_eq!(batches[0].schema().field(0).name(), "price");
            assert_eq!(batches[0].schema().field(1).name(), "id");
        }

        #[test]
        fn an_absent_resource_narrows_its_declared_schema_too() {
            let handle = handle("absent.arrows");
            let options = handle.record_options().unwrap().with_field(narrow());

            let reader = handle.read_arrow_reader(&options).unwrap();
            assert_eq!(reader.schema().fields().len(), 2);
            assert_eq!(reader.count(), 0);
        }
    }
}

/// The page cache as a handle: a cursor over it, and a real file under it.
///
/// The unit tests in [`crate::buffered`] cover the caching rules themselves;
/// these cover the wrapper where it meets the rest of `io`.
mod buffered_handle {
    use std::io::{Read, Seek, SeekFrom, Write};

    use super::{Buffer, IOBase};
    use crate::buffered::BufferedOptions;
    use crate::buffered::tests::Counting;
    use crate::io::IOCursor;

    /// Small pages, so a modest fixture crosses several of them.
    const PAGE: usize = 64;

    fn options() -> BufferedOptions {
        BufferedOptions::default()
            .with_page_size(PAGE)
            .with_max_bytes(16 * PAGE as u64)
    }

    /// A counting handle holding `size` bytes that name their own offset.
    fn counted(size: usize) -> Counting {
        Counting::from_bytes((0..size).map(|index| index as u8).collect())
    }

    #[test]
    fn a_cursor_over_a_buffered_handle_streams_across_pages() {
        let mut cursor = counted(4 * PAGE).buffered(options()).cursor();

        // Sixteen sequential reads over four pages: one inner read per page,
        // because the cursor rides the cache rather than the handle.
        let mut streamed = Vec::new();
        let mut chunk = [0_u8; 16];
        while cursor.read(&mut chunk).unwrap() == 16 {
            streamed.extend_from_slice(&chunk);
        }
        assert_eq!(streamed.len(), 4 * PAGE);
        assert_eq!(streamed[PAGE + 1], (PAGE + 1) as u8);
        assert_eq!(cursor.handle().handle().reads(), 4);
        assert_eq!(cursor.tell(), 4 * PAGE as u64);
    }

    #[test]
    fn a_cursor_seeks_to_the_end_without_re_reading() {
        let mut cursor = counted(4 * PAGE).buffered(options()).cursor();

        let mut head = [0_u8; 8];
        cursor.read_exact(&mut head).unwrap();
        assert_eq!(head[0], 0);
        Seek::seek(&mut cursor, SeekFrom::End(-8)).unwrap();

        let mut tail = [0_u8; 8];
        cursor.read_exact(&mut tail).unwrap();
        assert_eq!(tail[7], (4 * PAGE - 1) as u8);
        let reads = cursor.handle().handle().reads();

        // Both ends are pinned pages now, so going back to either is free.
        Seek::seek(&mut cursor, SeekFrom::Start(0)).unwrap();
        cursor.read_exact(&mut head).unwrap();
        Seek::seek(&mut cursor, SeekFrom::End(-8)).unwrap();
        cursor.read_exact(&mut tail).unwrap();
        assert_eq!(cursor.handle().handle().reads(), reads);

        // A seek past the end reads nothing, exactly as `pread` does.
        Seek::seek(&mut cursor, SeekFrom::End(64)).unwrap();
        assert_eq!(cursor.read(&mut tail).unwrap(), 0);
    }

    #[test]
    fn a_cursor_writes_through_and_reads_back_what_it_wrote() {
        let mut cursor = Buffer::from_bytes(vec![7_u8; 4 * PAGE])
            .buffered(options())
            .cursor();

        cursor.read_exact(&mut [0_u8; 8]).unwrap();
        cursor.seek_to(PAGE as u64 - 2);
        cursor.write_all(b"ABCD").unwrap();

        cursor.seek_to(PAGE as u64 - 2);
        let mut written = [0_u8; 4];
        cursor.read_exact(&mut written).unwrap();
        assert_eq!(&written, b"ABCD");
        assert_eq!(cursor.size(), 4 * PAGE as u64);
    }

    #[test]
    fn a_buffered_file_reads_from_pages_and_writes_through() {
        let path = std::env::temp_dir().join(format!(
            "yggdryl-buffered-file-{}-{:?}.bin",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_file(&path);
        let payload: Vec<u8> = (0..5_000_u32).map(|index| index as u8).collect();
        std::fs::write(&path, &payload).unwrap();

        let mut handle = crate::local::File::new(&path)
            .unwrap()
            .buffered(BufferedOptions::default().with_page_size(512));

        // The wrapper answers for the file it wraps.
        assert_eq!(handle.size(), 5_000);
        assert_eq!(handle.kind(), crate::IOKind::File);
        assert_eq!(
            handle.url().unwrap().file_name(),
            path.file_name().and_then(std::ffi::OsStr::to_str)
        );
        assert_eq!(handle.read_all_bytes().unwrap(), payload);
        assert_eq!(handle.cached_pages(), 10);

        // A write lands in the file and in the pages that held those bytes.
        handle.pwrite(600, b"trade").unwrap();
        handle.flush().unwrap();
        assert_eq!(handle.read_range_bytes(600, 5).unwrap(), b"trade");
        assert_eq!(std::fs::read(&path).unwrap()[600..605], *b"trade");

        // Closing releases the pages and leaves a working handle behind.
        handle.close().unwrap();
        assert_eq!(handle.cached_pages(), 0);
        assert_eq!(handle.read_range_bytes(600, 5).unwrap(), b"trade");

        drop(handle);
        let _ = std::fs::remove_file(&path);
    }
}

/// The byte contract, run over every backend rather than over one.
///
/// The assertions above describe `Buffer`, which is the implementation they
/// were written against. They are not *about* `Buffer` though - they are the
/// behavior [`IOBase`] requires of anything that holds bytes, so a backend
/// that passes them is a backend a caller can hand to `Coded`, `Ipc`,
/// `Parquet`, or an Iceberg table without reading its source. This module
/// runs one battery over each: the in-memory `Buffer`, the memory-mapped
/// `local::File`, an `arrowfs::File` over a foreign Arrow filesystem, whose
/// bytes are staged and published rather than written in place, and a
/// `Buffered` page cache over a `Buffer` - a wrapping handle rather than a
/// backend, but one whose whole claim is that it changes nothing a caller can
/// observe, which is exactly what this battery asks.
mod conformance {
    use super::{Buffer, IOBase};

    use std::sync::Arc;

    /// Every backend under test, each as a freshly built empty handle.
    ///
    /// The handles are boxed because the battery is one function rather than
    /// one per backend; `IOBase` is implemented for the box, so the byte half
    /// of the contract forwards unchanged.
    fn backends(label: &str) -> Vec<(&'static str, Box<dyn IOBase>)> {
        let mut root = std::env::temp_dir();
        root.push(format!(
            "yggdryl-conformance-{label}-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).expect("a writable temporary root");

        let memory = Arc::new(crate::arrowfs::MemoryFileSystem::new());
        vec![
            ("buffer", Box::new(Buffer::new()) as Box<dyn IOBase>),
            (
                "local::File",
                Box::new(
                    crate::local::File::create(root.join(format!("{label}.bin")))
                        .expect("a valid path"),
                ),
            ),
            (
                "arrowfs::File",
                Box::new(
                    crate::arrowfs::File::from_location(memory, &format!("bench/{label}.bin"))
                        .expect("a valid location"),
                ),
            ),
            (
                "buffered",
                Box::new(
                    // Pages far smaller than the default, so even these short
                    // fixtures cross page boundaries and exercise the cache
                    // rather than living inside one page.
                    Buffer::new()
                        .buffered(crate::buffered::BufferedOptions::default().with_page_size(4)),
                ),
            ),
        ]
    }

    /// Remove whatever the local backend left behind.
    fn cleanup(label: &str) {
        let mut root = std::env::temp_dir();
        root.push(format!(
            "yggdryl-conformance-{label}-{}",
            std::process::id()
        ));
        // Teardown goes through the abstraction, not around it: a folder
        // handle already addresses this tree, and absence is a no-op success.
        if let Ok(mut folder) = crate::local::Folder::new(&root) {
            folder.remove(true).expect("a removable tree");
        }
    }

    #[test]
    fn every_backend_grows_and_zero_fills_a_write_gap() {
        for (name, mut handle) in backends("gap") {
            handle.pwrite(0, b"trade").expect("a writable handle");
            assert_eq!(handle.size(), 5, "{name}");

            // Writing past the end grows the value and zero-fills the gap.
            handle.pwrite(8, b"!").expect("a writable handle");
            assert_eq!(handle.size(), 9, "{name}");
            assert_eq!(
                handle.read_all_bytes().expect("a readable handle"),
                b"trade\0\0\0!",
                "{name}"
            );
        }
        cleanup("gap");
    }

    #[test]
    fn every_backend_reads_positionally_without_a_shared_cursor() {
        for (name, mut handle) in backends("cursor") {
            handle
                .write_all_bytes(b"0123456789")
                .expect("a writable handle");

            // Two reads at different offsets, in either order.
            let mut tail = [0_u8; 3];
            handle.pread(7, &mut tail).expect("a readable handle");
            let mut head = [0_u8; 3];
            handle.pread(0, &mut head).expect("a readable handle");
            assert_eq!(&head, b"012", "{name}");
            assert_eq!(&tail, b"789", "{name}");

            // Entirely past the end is empty; straddling the end is short.
            let mut past = [0_u8; 4];
            assert_eq!(
                handle.pread(100, &mut past).expect("a readable handle"),
                0,
                "{name}"
            );
            assert_eq!(
                handle.pread(8, &mut past).expect("a readable handle"),
                2,
                "{name}"
            );
        }
        cleanup("cursor");
    }

    #[test]
    fn every_backend_reads_a_missing_resource_as_empty() {
        // The laziness contract: absence is emptiness on the read path, so a
        // caller probes a location without an existence check first.
        let memory = Arc::new(crate::arrowfs::MemoryFileSystem::new());
        let mut root = std::env::temp_dir();
        root.push(format!("yggdryl-conformance-absent-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);

        let absent: Vec<(&str, Box<dyn IOBase>)> = vec![
            (
                "local::File",
                Box::new(crate::local::File::new(root.join("absent.bin")).expect("a valid path")),
            ),
            (
                "arrowfs::File",
                Box::new(
                    crate::arrowfs::File::from_location(memory, "nowhere/absent.bin")
                        .expect("a valid location"),
                ),
            ),
        ];
        for (name, handle) in absent {
            assert_eq!(handle.size(), 0, "{name}");
            assert!(handle.is_empty(), "{name}");
            let mut probe = [0_u8; 8];
            assert_eq!(
                handle.pread(0, &mut probe).expect("a readable handle"),
                0,
                "{name}"
            );
            assert!(
                handle
                    .read_all_bytes()
                    .expect("a readable handle")
                    .is_empty(),
                "{name}"
            );
        }
        // Reading created nothing.
        assert!(!root.exists());
    }

    #[test]
    fn every_backend_truncates_shrinking_and_extending() {
        for (name, mut handle) in backends("truncate") {
            handle
                .write_all_bytes(b"0123456789")
                .expect("a writable handle");

            handle.truncate(4).expect("a resizable handle");
            assert_eq!(
                handle.read_all_bytes().expect("a readable handle"),
                b"0123",
                "{name}"
            );

            // Extending zero-fills rather than leaving stale bytes visible.
            handle.truncate(6).expect("a resizable handle");
            assert_eq!(
                handle.read_all_bytes().expect("a readable handle"),
                b"0123\0\0",
                "{name}"
            );

            handle.clear().expect("a clearable handle");
            assert!(handle.is_empty(), "{name}");
        }
        cleanup("truncate");
    }

    #[test]
    fn every_backend_keeps_capacity_at_or_above_size() {
        for (name, mut handle) in backends("capacity") {
            handle.reserve(4_096).expect("a reservable handle");
            assert!(handle.capacity() >= 4_096, "{name}");
            assert_eq!(handle.size(), 0, "{name}");

            handle.pwrite(0, b"x").expect("a writable handle");
            assert!(handle.capacity() >= handle.size(), "{name}");

            // The invariant holds after a shrink too, not only after growth.
            handle
                .write_all_bytes(&vec![7_u8; 8_192])
                .expect("a writable handle");
            assert!(handle.capacity() >= handle.size(), "{name}");
            handle.truncate(16).expect("a resizable handle");
            assert!(handle.capacity() >= handle.size(), "{name}");
        }
        cleanup("capacity");
    }

    #[test]
    fn every_backend_appends_where_it_says_it_did() {
        for (name, mut handle) in backends("append") {
            assert_eq!(
                handle.append_bytes(b"first").expect("a writable handle"),
                0,
                "{name}"
            );
            assert_eq!(
                handle.append_bytes(b"second").expect("a writable handle"),
                5,
                "{name}"
            );
            assert_eq!(
                handle.read_all_bytes().expect("a readable handle"),
                b"firstsecond",
                "{name}"
            );
        }
        cleanup("append");
    }

    #[test]
    fn every_backend_replaces_the_whole_value_and_bounds_a_range_read() {
        for (name, mut handle) in backends("replace") {
            handle
                .write_all_bytes(b"a much longer previous value")
                .expect("a writable handle");
            handle.write_all_bytes(b"short").expect("a writable handle");
            assert_eq!(
                handle.read_all_bytes().expect("a readable handle"),
                b"short",
                "{name}"
            );
            assert_eq!(handle.size(), 5, "{name}");

            handle
                .write_all_bytes(b"0123456789")
                .expect("a writable handle");
            assert_eq!(
                handle.read_range_bytes(2, 3).expect("a readable handle"),
                b"234",
                "{name}"
            );
            // Asking past the end yields what exists rather than failing.
            assert_eq!(
                handle.read_range_bytes(8, 100).expect("a readable handle"),
                b"89",
                "{name}"
            );
            assert!(
                handle
                    .read_range_bytes(50, 4)
                    .expect("a readable handle")
                    .is_empty(),
                "{name}"
            );
        }
        cleanup("replace");
    }

    #[test]
    fn every_backend_names_the_shortfall_of_an_exact_read() {
        for (name, mut handle) in backends("exact") {
            handle.write_all_bytes(b"abc").expect("a writable handle");
            let mut target = [0_u8; 8];
            let message = handle
                .pread_exact(0, &mut target)
                .expect_err("a short value cannot fill the buffer")
                .to_string();
            assert!(message.contains("expected 8 bytes"), "{name}: {message}");
            assert!(message.contains("got 3"), "{name}: {message}");
        }
        cleanup("exact");
    }

    #[test]
    fn every_backend_copies_into_every_other_one() {
        // The transfer is chunked through the trait alone, so a copy works
        // in every direction across backends without either side knowing
        // what the other is.
        for (source_name, mut source) in backends("copy-source") {
            source
                .write_all_bytes(b"symbol,price\nAAPL,1\n")
                .expect("a writable handle");
            for (target_name, mut target) in backends("copy-target") {
                target
                    .write_all_bytes(b"stale contents")
                    .expect("a writable handle");
                let copied = source.copy_into(target.as_mut()).expect("a copyable pair");

                assert_eq!(copied, source.size(), "{source_name} -> {target_name}");
                assert_eq!(
                    target.read_all_bytes().expect("a readable handle"),
                    source.read_all_bytes().expect("a readable handle"),
                    "{source_name} -> {target_name}"
                );
            }
            cleanup("copy-target");
        }
        cleanup("copy-source");
    }

    #[test]
    fn every_backend_streams_through_the_reader_and_writer_adapters() {
        use std::io::{Read, Write};

        for (name, mut handle) in backends("streams") {
            {
                let mut writer = handle.writer_at(0);
                writer.write_all(b"symbol,").expect("a writable adapter");
                writer.write_all(b"price").expect("a writable adapter");
                writer.flush().expect("a flushable adapter");
            }
            assert_eq!(
                handle.read_all_bytes().expect("a readable handle"),
                b"symbol,price",
                "{name}"
            );

            let mut text = String::new();
            handle
                .reader_at(0)
                .read_to_string(&mut text)
                .expect("a readable adapter");
            assert_eq!(text, "symbol,price", "{name}");

            // A reader can start anywhere without disturbing another.
            let mut tail = String::new();
            handle
                .reader_at(7)
                .read_to_string(&mut tail)
                .expect("a readable adapter");
            assert_eq!(tail, "price", "{name}");
        }
        cleanup("streams");
    }

    #[test]
    fn every_backend_round_trips_a_content_coding() {
        for (name, mut handle) in backends("coding") {
            let payload = "symbol,price\n".repeat(500).into_bytes();
            handle.write_all_bytes(&payload).expect("a writable handle");

            for codec in [crate::Codec::Gzip, crate::Codec::Zlib, crate::Codec::Zstd] {
                let mut compressed = Buffer::new();
                handle
                    .compress_into(&mut compressed, codec)
                    .expect("an encodable value");
                assert!(compressed.size() < handle.size(), "{name}/{codec}");
                assert_eq!(compressed.codec(), codec, "{name}/{codec}");

                let mut restored = Buffer::new();
                compressed
                    .decompress_into(&mut restored)
                    .expect("a decodable value");
                assert_eq!(restored.as_slice(), payload.as_slice(), "{name}/{codec}");
            }
        }
        cleanup("coding");
    }
}

/// The lifecycle pair: `clear` empties, `remove` deletes, absence is a no-op
/// success reached without a probe.
mod lifecycle {
    use super::*;
    #[cfg(feature = "arrow")]
    use crate::io::IOMedia;
    use crate::local::{File, Folder, Path};

    /// A temp root nothing else in this file uses.
    fn root(label: &str) -> std::path::PathBuf {
        let mut root = std::env::temp_dir();
        root.push(format!("yggdryl-lifecycle-{label}-{}", std::process::id()));
        let mut folder = Folder::new(&root).expect("a local container");
        folder.remove(true).expect("a removable tree");
        root
    }

    /// A handle counting every method a probe would go through.
    ///
    /// The point is the assertion this makes possible: `remove` on an absent
    /// resource must issue exactly one delete attempt and *zero* probes. A
    /// future `if self.kind() == IOKind::Unknown` convenience guard would be
    /// invisible in prose and obvious here.
    #[derive(Default)]
    struct Counted {
        bytes: Buffer,
        kinds: std::cell::Cell<usize>,
        sizes: std::cell::Cell<usize>,
        listings: std::cell::Cell<usize>,
        deletes: std::cell::Cell<usize>,
        clears: std::cell::Cell<usize>,
    }

    // Written out rather than delegated, because every method the delegation
    // would supply is one this double has to count. The counters are
    // per-handle and never shared across threads; `IOBase` requires `Send`,
    // and `Cell` is `Send` when its contents are.
    impl crate::io::IOMedia for Counted {
        crate::impl_default_iomedia!();
    }

    impl IOBase for Counted {
        fn pread(&self, offset: u64, buffer: &mut [u8]) -> crate::Result<usize> {
            self.bytes.pread(offset, buffer)
        }

        fn pwrite(&mut self, offset: u64, bytes: &[u8]) -> crate::Result<usize> {
            self.bytes.pwrite(offset, bytes)
        }

        fn capacity(&self) -> u64 {
            self.bytes.capacity()
        }

        fn reserve(&mut self, capacity: u64) -> crate::Result<()> {
            self.bytes.reserve(capacity)
        }

        fn truncate(&mut self, size: u64) -> crate::Result<()> {
            self.bytes.truncate(size)
        }

        fn url(&self) -> Option<&crate::Url> {
            self.bytes.url()
        }

        fn media_type(&self) -> &crate::MediaType {
            self.bytes.media_type()
        }

        fn set_media_type(&mut self, media_type: crate::MediaType) {
            self.bytes.set_media_type(media_type);
        }

        fn kind(&self) -> crate::IOKind {
            self.kinds.set(self.kinds.get() + 1);
            crate::IOKind::Memory
        }

        fn size(&self) -> u64 {
            self.sizes.set(self.sizes.get() + 1);
            self.bytes.size()
        }

        fn ls(&self, _recursive: bool, _include_private: bool) -> crate::io::Listing {
            self.listings.set(self.listings.get() + 1);
            crate::io::Listing::empty()
        }

        fn clear(&mut self) -> crate::Result<()> {
            self.clears.set(self.clears.get() + 1);
            self.bytes.clear()
        }

        fn remove(&mut self, recursive: bool) -> crate::Result<()> {
            // Exactly what every backend does: issue the delete, treat the
            // store's own not-found answer as success, probe nothing first.
            self.deletes.set(self.deletes.get() + 1);
            self.bytes.remove(recursive)
        }
    }

    #[test]
    fn removing_an_absent_resource_issues_one_delete_and_no_probe() {
        let mut handle = Counted::default();
        handle.remove(false).expect("absence is a no-op success");

        assert_eq!(handle.deletes.get(), 1, "exactly one delete attempt");
        assert_eq!(handle.kinds.get(), 0, "no kind() probe");
        assert_eq!(handle.sizes.get(), 0, "no size() probe");
        assert_eq!(handle.listings.get(), 0, "no ls() probe");

        handle.clear().expect("absence is a no-op success");
        assert_eq!(handle.clears.get(), 1);
        assert_eq!(handle.kinds.get(), 0, "no kind() probe");
        assert_eq!(handle.sizes.get(), 0, "no size() probe");
    }

    #[test]
    fn a_leaf_clears_empty_and_removes_gone() {
        let root = root("leaf");
        let path = root.join("trades.csv");
        let mut leaf = File::new(&path).expect("a local leaf");
        leaf.write_all_bytes(b"symbol,price\n").expect("a write");
        leaf.flush().expect("a flush");

        leaf.clear().expect("a clearable leaf");
        assert_eq!(leaf.size(), 0, "cleared to empty");
        assert!(path.exists(), "the resource still exists after clear");

        leaf.remove(false).expect("a removable leaf");
        assert!(!path.exists(), "removed");

        // Both succeed a second time: absence is never an error.
        leaf.clear().expect("clearing an absent leaf");
        assert!(!path.exists(), "clearing an absent leaf never creates it");
        leaf.remove(false).expect("removing an absent leaf");

        // The handle stays usable and lazy - a write recreates the resource.
        leaf.write_all_bytes(b"MSFT,2").expect("a write");
        leaf.flush().expect("a flush");
        assert_eq!(leaf.read_all_bytes().expect("a read"), b"MSFT,2");

        Folder::new(&root).expect("a container").remove(true).ok();
    }

    #[test]
    fn a_container_clears_empty_and_removes_by_recursion() {
        let root = root("container");
        let mut folder = Folder::new(&root).expect("a local container");
        folder.truncate(0).expect("a created container");
        for name in ["a.log", "b.log"] {
            folder
                .child_by_path(name)
                .expect("a child")
                .write_all_bytes(b"line\n")
                .expect("a write");
        }
        let mut nested = Folder::new(root.join("deep")).expect("a local container");
        nested.truncate(0).expect("a created container");
        nested
            .child_by_path("c.log")
            .expect("a child")
            .write_all_bytes(b"line\n")
            .expect("a write");

        folder.clear().expect("a clearable container");
        assert!(root.exists(), "the container still exists after clear");
        assert!(
            folder
                .ls(true, true)
                .collect::<crate::Result<Vec<_>>>()
                .expect("a listing")
                .is_empty(),
            "and is empty"
        );

        // An empty container is removable without recursion.
        folder.remove(false).expect("an empty container removes");
        assert!(!root.exists());

        // A populated one is not.
        folder.truncate(0).expect("a created container");
        folder
            .child_by_path("a.log")
            .expect("a child")
            .write_all_bytes(b"line\n")
            .expect("a write");
        let refused = folder.remove(false).expect_err("a populated container");
        let message = refused.to_string();
        assert!(message.contains("children"), "{message}");
        assert!(
            message.contains(root.file_name().expect("a name").to_string_lossy().as_ref()),
            "the refusal names the location: {message}"
        );
        assert!(root.exists(), "and nothing was deleted");

        folder.remove(true).expect("a recursive removal");
        assert!(!root.exists());
        folder.remove(true).expect("absence is a no-op success");
    }

    #[test]
    fn a_generic_path_routes_on_the_kind_it_already_resolved() {
        let root = root("path");
        Folder::new(&root)
            .expect("a container")
            .truncate(0)
            .expect("a created container");

        let leaf = root.join("one.log");
        Path::new(&leaf)
            .expect("a location")
            .write_all_bytes(b"line\n")
            .expect("a write");

        let mut path = Path::new(&leaf).expect("a location");
        assert_eq!(path.kind(), crate::IOKind::File);
        path.remove(false).expect("a removable leaf");
        assert!(!leaf.exists());

        let mut container = Path::new(&root).expect("a location");
        assert_eq!(container.kind(), crate::IOKind::Directory);
        container.remove(true).expect("a removable container");
        assert!(!root.exists());

        // Undecided is absence, which is a no-op success.
        let mut absent = Path::new(root.join("never")).expect("a location");
        assert_eq!(absent.kind(), crate::IOKind::Unknown);
        absent.clear().expect("a no-op clear");
        absent.remove(true).expect("a no-op removal");
    }

    #[test]
    fn a_pending_write_cannot_survive_a_removal() {
        let root = root("pending");
        let path = root.join("staged.bin");
        let mut leaf = File::new(&path).expect("a local leaf");

        // Write, do not flush, remove, then flush.
        leaf.pwrite(0, b"unflushed").expect("a write");
        leaf.remove(false).expect("a removable leaf");
        leaf.flush().expect("a flush after removal");

        assert!(
            !path.exists(),
            "a flush after a removal must not recreate the resource"
        );
        Folder::new(&root).expect("a container").remove(true).ok();
    }

    #[test]
    fn a_buffer_gives_its_allocation_back() {
        let mut buffer = Buffer::from_bytes(vec![7_u8; 4096]);
        buffer.clear().expect("a clearable buffer");
        assert_eq!(buffer.size(), 0);
        assert!(
            buffer.capacity() >= 4096,
            "clearing keeps the allocation for the next write"
        );

        buffer.remove(false).expect("a removable buffer");
        assert_eq!(buffer.size(), 0);
        assert_eq!(buffer.capacity(), 0, "removing gives the memory back");

        // Still usable and lazy afterwards.
        buffer.write_all_bytes(b"AAPL").expect("a write");
        assert_eq!(buffer.read_all_bytes().expect("a read"), b"AAPL");
    }

    #[test]
    fn a_coding_handle_removes_the_encoded_resource() {
        let root = root("coded");
        let path = root.join("trades.csv.gz");
        let mut coded = crate::gzip::Gzip::new(File::new(&path).expect("a local leaf"));
        coded
            .write_all_bytes(b"symbol,price\n")
            .expect("a decoded write");
        coded.close().expect("a published value");
        assert!(path.exists(), "the encoded bytes are on disk");

        coded.remove(false).expect("a removable coded resource");
        assert!(!path.exists(), "the .gz resource itself is gone");
        assert_eq!(coded.read_all_bytes().expect("a read").len(), 0);

        // A later flush must not resurrect it from a held decoded buffer.
        coded.flush().expect("a flush after removal");
        assert!(!path.exists());

        Folder::new(&root).expect("a container").remove(true).ok();
    }

    #[cfg(feature = "arrow")]
    #[test]
    fn a_media_handle_drops_its_cache_as_part_of_the_removal() {
        use crate::arrow::batch_reader;
        use crate::ipc::Ipc;

        let field = crate::DataType::from_fields([crate::DataType::Int64.required_field("id")])
            .expect("a struct root")
            .required_field("row");
        let schema = crate::arrow::arrow_schema_from_field(&field).expect("an Arrow schema");

        let mut media = Ipc::new(Buffer::new());
        let options = media.record_options().expect("IPC options");
        media
            .overwrite_arrow_reader(batch_reader(schema, []), &options)
            .expect("a write");
        media.open().expect("an opened media");
        assert!(media.opened(), "the schema is cached");

        media.remove(false).expect("a removable media");
        assert!(
            !media.opened(),
            "the cache is invalidated as part of the removal, not on the next open"
        );
        assert_eq!(media.size(), 0, "and the encoded bytes are gone");
    }
}

/// Which of the two surfaces a handle answers: bytes whole, or rows.
mod shape {
    use super::{Buffer, IOBase};

    use crate::buffered::tests::Counting;
    use crate::generic::Holder;
    use crate::io::Coded;
    use crate::{Codec, IOKind, MediaType, MimeType};

    /// A writable temporary root of this test's own.
    fn root(label: &str) -> std::path::PathBuf {
        let mut path = std::env::temp_dir();
        path.push(format!("yggdryl-shape-{label}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).expect("a writable temporary root");
        path
    }

    #[test]
    fn a_leaf_answers_from_its_representation_and_the_two_are_complements() {
        for (mime, tabular) in [
            (MimeType::PLAIN_TEXT, false),
            (MimeType::JSON, false),
            (MimeType::PARQUET, true),
            (MimeType::ARROW_FILE, true),
            (MimeType::CSV, true),
        ] {
            let mut handle = Buffer::new();
            handle.set_media_type(MediaType::from(mime.clone()));
            assert_eq!(handle.is_tabular(), tabular, "{mime}");
            // Exactly one of the two, because a leaf is read one way or the
            // other and never both.
            assert_eq!(handle.is_atomic(), !tabular, "{mime}");
            assert!(handle.is_io(), "{mime}");
        }
    }

    #[test]
    fn a_default_buffer_is_one_whole_byte_value() {
        let handle = Buffer::from_bytes(b"AAPL".to_vec());
        assert_eq!(handle.kind(), IOKind::Memory);
        assert!(handle.is_atomic());
        assert!(!handle.is_tabular());
    }

    #[test]
    fn a_content_coding_answers_for_the_representation_underneath_it() {
        // `trades.arrows.gz` is an Arrow file that happens to be compressed,
        // so the coding never changes which surface reads it.
        let media = MediaType::from_file_name("trades.arrows.gz");
        assert_eq!(media.base(), &MimeType::ARROW_STREAM);
        assert_eq!(media.encodings(), [MimeType::GZIP]);
        let mut handle = Buffer::new();
        handle.set_media_type(media);
        assert!(handle.is_tabular());
        assert!(!handle.is_atomic());

        let coded = Coded::new(handle, Codec::Gzip);
        assert!(coded.is_tabular());
        assert!(!coded.is_atomic());
    }

    #[test]
    fn a_named_location_answers_before_anything_exists() {
        let path = root("named");

        // Nothing has been written, so the kind is undecided - and the name
        // still says which surface reads it, exactly as the media type does.
        let missing = crate::local::Path::new(path.join("trades.parquet")).unwrap();
        assert_eq!(missing.kind(), IOKind::Unknown);
        assert_eq!(missing.media_type().base(), &MimeType::PARQUET);
        assert!(missing.is_tabular());
        assert!(!missing.is_atomic());

        let notes = crate::local::Path::new(path.join("notes.txt")).unwrap();
        assert_eq!(notes.kind(), IOKind::Unknown);
        assert!(notes.is_atomic());
        assert!(!notes.is_tabular());

        // The leaf implementation answers the same, existing or not.
        let leaf = crate::local::File::new(path.join("trades.arrows")).unwrap();
        assert!(leaf.is_tabular());
        assert!(!leaf.is_atomic());

        let _ = std::fs::remove_dir_all(&path);
    }

    #[test]
    fn a_folder_reads_as_the_table_beneath_it() {
        let path = root("folder");
        let lake = path.join("lake");
        std::fs::create_dir_all(lake.join("year=2024/month=01")).unwrap();
        std::fs::write(lake.join("year=2024/month=01/part-0.parquet"), b"PAR1").unwrap();

        let folder = crate::local::Folder::new(&lake).unwrap();
        assert_eq!(folder.kind(), IOKind::Directory);
        assert!(folder.is_container());
        // The probe descends to the first leaf; a folder is never one whole
        // byte value whatever is under it.
        assert!(folder.is_tabular());
        assert!(!folder.is_atomic());

        // A container of plain files is neither: no rows to read, and no one
        // byte value to read whole.
        let logs = path.join("logs");
        std::fs::create_dir_all(&logs).unwrap();
        std::fs::write(logs.join("run.txt"), b"started").unwrap();
        let folder = crate::local::Folder::new(&logs).unwrap();
        assert!(!folder.is_tabular());
        assert!(!folder.is_atomic());
        assert!(!folder.is_io());

        // So is an empty one, and so is a folder that does not exist yet.
        let empty = crate::local::Folder::new(path.join("empty")).unwrap();
        assert!(!empty.is_tabular());
        assert!(!empty.is_atomic());
        assert!(!empty.is_io());

        // A location resolving to that lake answers exactly as the folder did.
        let located = crate::local::Path::new(&lake).unwrap();
        assert_eq!(located.kind(), IOKind::Directory);
        assert!(located.is_tabular());
        assert!(!located.is_atomic());

        let _ = std::fs::remove_dir_all(&path);
    }

    #[test]
    fn a_record_encoding_handle_answers_without_touching_its_bytes() {
        // The buffer underneath carries no media type at all, so nothing but
        // the encoding itself can be answering here.
        let plain = Buffer::new();
        assert!(plain.is_atomic());

        let ipc = crate::ipc::Ipc::new(Buffer::new());
        assert!(ipc.is_tabular());
        assert!(!ipc.is_atomic());

        #[cfg(feature = "parquet")]
        {
            let parquet = crate::parquet::Parquet::new(Buffer::new());
            assert!(parquet.is_tabular());
            assert!(!parquet.is_atomic());
        }

        let avro = crate::avro::Avro::new(Buffer::new());
        assert!(avro.is_tabular());
        assert!(!avro.is_atomic());
    }

    #[test]
    fn asking_the_shape_of_a_leaf_reads_nothing() {
        // The counting double is the measuring instrument the page cache uses:
        // it reports every `pread` and every `size` that reaches the bytes.
        let mut handle = Counting::from_bytes(b"PAR1".to_vec());
        handle.set_media_type(MediaType::from(MimeType::PARQUET));

        assert!(handle.is_tabular());
        assert!(!handle.is_atomic());

        // Both answers came from the representation, so nothing was read and
        // nothing was even measured.
        assert_eq!(handle.reads(), 0);
        assert_eq!(handle.sizes(), 0);
    }

    #[test]
    fn wrapping_a_handle_keeps_the_shape_it_wraps() {
        let mut handle = Buffer::new();
        handle.set_media_type(MediaType::from(MimeType::PARQUET));

        // A page cache is invisible: it answers exactly what it wraps.
        let cached = handle.buffered(crate::buffered::BufferedOptions::default());
        assert!(cached.is_tabular());
        assert!(!cached.is_atomic());

        // So is the generic enum every listing hands back.
        let held = Holder::from(Buffer::from_bytes(b"AAPL".to_vec()));
        assert!(held.is_atomic());
        assert!(!held.is_tabular());
    }

    #[cfg(feature = "arrow")]
    #[test]
    fn folder_dimensions_sum_only_the_selected_record_encoding() {
        use std::sync::Arc;

        use arrow_array::{Int64Array, RecordBatch};

        use crate::io::IOMedia as _;

        fn rows(values: &[i64]) -> RecordBatch {
            let schema = Arc::new(arrow_schema::Schema::new(vec![arrow_schema::Field::new(
                "id",
                arrow_schema::DataType::Int64,
                false,
            )]));
            RecordBatch::try_new(schema, vec![Arc::new(Int64Array::from(values.to_vec()))])
                .expect("a dimension fixture")
        }

        let path = root("dimensions");
        let lake = path.join("lake");
        for (name, values) in [("a.arrows", vec![1, 2]), ("b.arrows", vec![3])] {
            let mut leaf = crate::local::Path::new(lake.join(name)).expect("a lazy leaf");
            let batch = rows(&values);
            let options = leaf.record_options().expect("IPC options");
            leaf.overwrite_arrow_reader(
                crate::arrow::batch_reader(batch.schema(), [batch]),
                &options,
            )
            .expect("a published IPC leaf");
        }
        crate::local::Path::new(lake.join("notes.txt"))
            .expect("a text leaf")
            .write_all_bytes(b"not a table row")
            .expect("a published unrelated leaf");

        let folder = crate::local::Folder::new(&lake).expect("the lake folder");
        assert_eq!(folder.row_size().expect("metadata row count"), 3);
        assert_eq!(folder.column_size().expect("metadata field width"), 1);

        let _ = std::fs::remove_dir_all(&path);
    }
}

/// A listing must never require holding all of it, and the way to prove that is
/// to count what the backend was actually asked for.
mod laziness {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::super::{IOBase, Listing};
    use crate::generic::Holder;
    use crate::{Error, IOKind, MediaType, MimeType, Result, Url};

    /// A container of `width` synthetic leaves that counts what it produces.
    ///
    /// `opened` counts directory reads, `produced` counts entries actually
    /// materialized. A lazy listing moves the second number by exactly what the
    /// caller drained; an eager one moves it by `width` whatever the caller did.
    #[derive(Debug, Clone)]
    struct Wide {
        url: Url,
        width: usize,
        /// Directories this handle was asked to read.
        opened: Arc<AtomicUsize>,
        /// Entries this handle actually materialized.
        produced: Arc<AtomicUsize>,
        /// The zero-based entry index the listing fails at, if any.
        fails_at: Option<usize>,
    }

    impl Wide {
        fn new(width: usize) -> Self {
            Self {
                url: Url::from_str("memory://wide").expect("a valid location"),
                width,
                opened: Arc::new(AtomicUsize::new(0)),
                produced: Arc::new(AtomicUsize::new(0)),
                fails_at: None,
            }
        }

        fn failing_at(mut self, index: usize) -> Self {
            self.fails_at = Some(index);
            self
        }

        fn opened(&self) -> usize {
            self.opened.load(Ordering::Relaxed)
        }

        fn produced(&self) -> usize {
            self.produced.load(Ordering::Relaxed)
        }
    }

    impl crate::io::IOMedia for Wide {
        crate::impl_default_iomedia!();
    }

    impl IOBase for Wide {
        fn pread(&self, _offset: u64, _buffer: &mut [u8]) -> Result<usize> {
            Ok(0)
        }

        fn pwrite(&mut self, _offset: u64, bytes: &[u8]) -> Result<usize> {
            Ok(bytes.len())
        }

        fn size(&self) -> u64 {
            0
        }

        fn capacity(&self) -> u64 {
            0
        }

        fn reserve(&mut self, _capacity: u64) -> Result<()> {
            Ok(())
        }

        fn truncate(&mut self, _size: u64) -> Result<()> {
            Ok(())
        }

        fn url(&self) -> Option<&Url> {
            Some(&self.url)
        }

        fn media_type(&self) -> &MediaType {
            static DIRECTORY: std::sync::OnceLock<MediaType> = std::sync::OnceLock::new();
            DIRECTORY.get_or_init(|| MediaType::from(MimeType::DIRECTORY))
        }

        fn set_media_type(&mut self, _media_type: MediaType) {}

        fn kind(&self) -> IOKind {
            IOKind::Directory
        }

        fn ls(&self, _recursive: bool, _include_private: bool) -> Listing {
            let opened = Arc::clone(&self.opened);
            let produced = Arc::clone(&self.produced);
            let fails_at = self.fails_at;
            let root = self.url.clone();
            let width = self.width;
            // Deferred exactly as a real backend's is: the directory read
            // happens on the first `next`, not when the listing is built.
            Listing::new(std::iter::once(()).flat_map(move |()| {
                opened.fetch_add(1, Ordering::Relaxed);
                let produced = Arc::clone(&produced);
                let root = root.clone();
                Listing::new((0..width).map(move |index| {
                    produced.fetch_add(1, Ordering::Relaxed);
                    if fails_at == Some(index) {
                        return Err(Error::absent("file", format!("{root}/part-{index}")));
                    }
                    Ok(Holder::from(crate::io::Buffer::new()))
                }))
            }))
        }
    }

    #[test]
    fn building_a_listing_touches_nothing() {
        let wide = Wide::new(10_000);
        let listing = wide.ls(false, false);
        assert_eq!(wide.opened(), 0, "no directory read yet");
        assert_eq!(wide.produced(), 0, "no entry materialized yet");
        drop(listing);
        assert_eq!(wide.opened(), 0, "a listing nobody drained costs nothing");
    }

    #[test]
    fn taking_three_entries_costs_three_entries() {
        let wide = Wide::new(10_000);
        let taken: Vec<_> = wide
            .ls(false, false)
            .take(3)
            .collect::<Result<Vec<_>>>()
            .expect("three entries");

        assert_eq!(taken.len(), 3);
        assert_eq!(wide.opened(), 1, "one directory read");
        assert_eq!(
            wide.produced(),
            3,
            "exactly the three entries the caller asked for, not ten thousand"
        );
    }

    #[test]
    fn a_failing_entry_ends_the_listing_without_discarding_what_came_before() {
        let wide = Wide::new(10).failing_at(2);
        let entries: Vec<_> = wide.ls(false, false).collect();

        assert_eq!(entries.len(), 3, "two entries, then the failure");
        assert!(entries[0].is_ok());
        assert!(entries[1].is_ok());
        assert!(entries[2].as_ref().is_err_and(Error::is_absent));
        assert_eq!(
            wide.produced(),
            3,
            "the listing stopped at the failing entry rather than draining past it"
        );
    }

    #[test]
    fn the_same_listing_over_the_same_state_yields_the_same_order_twice() {
        let root = std::env::temp_dir().join(format!("yggdryl-order-{}", std::process::id()));
        let mut folder = crate::local::Folder::new(&root).expect("a local folder");
        folder.remove(true).ok();
        for name in ["c.bin", "a.bin", "b.bin"] {
            let mut leaf = folder.child_by_path(name).expect("a child");
            leaf.write_all_bytes(b"x").expect("a write");
        }

        let names = || -> Vec<String> {
            folder
                .ls(true, false)
                .map(|entry| {
                    Ok(entry?
                        .url()
                        .and_then(|url| url.file_name())
                        .unwrap_or_default()
                        .to_owned())
                })
                .collect::<Result<Vec<_>>>()
                .expect("a listing")
        };
        assert_eq!(names(), names());
        assert_eq!(names(), ["a.bin", "b.bin", "c.bin"]);

        folder.remove(true).expect("a removable folder");
    }

    #[test]
    fn a_glob_whose_fixed_prefix_loses_lists_nothing_beneath_it() {
        let root = std::env::temp_dir().join(format!("yggdryl-prefix-{}", std::process::id()));
        let mut folder = crate::local::Folder::new(&root).expect("a local folder");
        folder.remove(true).ok();
        let mut leaf = folder
            .child_by_path("year=2024/month=01/part-0.parquet")
            .expect("a child");
        leaf.write_all_bytes(b"PAR1").expect("a write");

        // The pattern is built but never drained, so nothing is read at all.
        let listing = folder
            .glob("year=1999/**/*.parquet", false)
            .expect("an expandable pattern");
        drop(listing);

        // Drained, it descends the fixed prefix, finds nothing there, and never
        // looks at `year=2024`.
        assert_eq!(
            folder
                .glob("year=1999/**/*.parquet", false)
                .expect("an expandable pattern")
                .count(),
            0
        );
        assert_eq!(
            folder
                .glob("year=2024/**/*.parquet", false)
                .expect("an expandable pattern")
                .count(),
            1
        );

        folder.remove(true).expect("a removable folder");
    }

    #[test]
    fn a_recursive_walk_descends_one_level_at_a_time() {
        // A deep, narrow tree: sixteen levels, each holding one directory and
        // one leaf. The walk yields an entry before the subtree under it, and
        // what it retains is one level's cursor per *open* level - the
        // frontier - never the thirty-two entries it will eventually yield.
        let root = std::env::temp_dir().join(format!("yggdryl-deep-{}", std::process::id()));
        let mut folder = crate::local::Folder::new(&root).expect("a local folder");
        folder.remove(true).ok();
        let mut path = String::new();
        for level in 0..16 {
            path.push_str(&format!("level-{level:02}/"));
            let mut leaf = folder
                .child_by_path(&format!("{path}leaf.bin"))
                .expect("a child");
            leaf.write_all_bytes(b"x").expect("a write");
        }

        let mut seen = 0_usize;
        for entry in folder.ls(true, false) {
            entry.expect("an entry");
            seen += 1;
        }
        assert_eq!(seen, 32, "sixteen directories and sixteen leaves");

        folder.remove(true).expect("a removable folder");
    }
}

mod applying {
    //! A target the expression module has never heard of.
    //!
    //! [`ApplyExpression`] inverts who owns evaluation: the target says how an
    //! expression applies to it. This listing lives here, outside
    //! `expression/`, and reaches that module through its public surface
    //! alone - if it ever needed a line inside `expression/` beyond a `use`,
    //! the trait would be shaped wrong.
    //!
    //! [`ApplyExpression`]: crate::expression::ApplyExpression

    use crate::expression::{ApplyExpression, Bound, Expression};
    use crate::{DataType, Result, Scalar, Url};

    /// An owned listing, seen as a target: applying a predicate yields the
    /// positions of the entries it does not rule out, in listing order.
    struct Listing(Vec<Url>);

    impl ApplyExpression for Listing {
        type Output = Vec<usize>;

        fn apply_expression(&self, bound: &Bound) -> Result<Vec<usize>> {
            let mut kept = Vec::new();
            for (position, entry) in self.0.iter().enumerate() {
                // A `Url` already answers holder attributes, so the listing
                // composes the public conservative verb rather than walking
                // the expression itself.
                if bound.matches_holder(entry)? {
                    kept.push(position);
                }
            }
            Ok(kept)
        }
    }

    #[test]
    fn a_listing_defined_outside_the_expression_module_is_a_target() {
        let listing = Listing(
            [
                "file:///lake/year=2024/part-0.parquet",
                "file:///lake/year=2023/part-0.parquet",
                "file:///lake/year=2024/part-1.parquet",
            ]
            .into_iter()
            .map(|url| Url::from_str(url).unwrap())
            .collect(),
        );
        // The same empty-column schema the holder filters in this module bind
        // against: the predicate reads no row, only the holder.
        let schema = DataType::from_fields([]).unwrap().required_field("holder");
        let bound = "&holder.partition['year'] = '2024'"
            .parse::<Expression>()
            .unwrap()
            .bind(&schema)
            .unwrap();
        assert_eq!(listing.apply_expression(&bound).unwrap(), vec![0, 2]);

        // A predicate a path cannot answer rules nothing out, exactly as the
        // holder target promises.
        let unknown = "&holder.size > 0"
            .parse::<Expression>()
            .unwrap()
            .bind(&schema)
            .unwrap();
        assert_eq!(listing.apply_expression(&unknown).unwrap(), vec![0, 1, 2]);
    }

    #[test]
    fn the_row_target_answers_through_the_same_trait() {
        // The trait is also how the built-in targets are reached: one row
        // applies to the value the expression computes.
        let schema = DataType::from_fields([DataType::Int64.named_field("i", true)])
            .unwrap()
            .required_field("row");
        let bound = "i + 1"
            .parse::<Expression>()
            .unwrap()
            .bind(&schema)
            .unwrap();
        let row = Scalar::from_sequence([Scalar::I64(41)]);
        assert_eq!(row.apply_expression(&bound).unwrap(), Scalar::I64(42));
    }
}
