//! Behavior every [`IOBase`] implementation must share.

use std::io::{Read, Write};

use super::{Buffer, IOBase};
use crate::Codec;
use crate::{MediaType, MimeType, Url};

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
    assert_eq!(buffer.append(b"first").unwrap(), 0);
    assert_eq!(buffer.append(b"second").unwrap(), 5);
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
    assert_eq!(buffer.read_range(2, 3).unwrap(), b"234");
    // Asking past the end yields what exists rather than failing.
    assert_eq!(buffer.read_range(8, 100).unwrap(), b"89");
    assert!(buffer.read_range(50, 4).unwrap().is_empty());
}

#[test]
fn write_all_bytes_replaces_the_whole_value() {
    let mut buffer = Buffer::from_bytes(b"a much longer previous value".to_vec());
    buffer.write_all_bytes(b"short").unwrap();
    assert_eq!(buffer.as_slice(), b"short");
    assert_eq!(buffer.size(), 5);
}

/// Any handle reads and writes Arrow batches through exactly three methods:
/// one read, one write, one append. The encoding comes from the handle's own
/// media type, and every one of the three takes or returns a batch reader.
#[cfg(feature = "arrow")]
mod records {
    use std::sync::Arc;

    use arrow_array::{Int64Array, RecordBatch, RecordBatchReader, StringArray};

    use crate::arrow::BatchReader;
    use crate::generic::{IORecordOptions, RecordOptions};
    use crate::io::{Buffer, IOBase};
    use crate::{DataType, Field, MimeType, Url};

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
            crate::arrow::schema_from_field(&schema()).unwrap(),
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
            crate::arrow::schema_from_field(&schema()).unwrap(),
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
            .read_arrow_batch_reader(options)
            .unwrap()
            .map(|batch| batch.unwrap().num_rows())
            .sum()
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
                .with_schema(schema())
                .with_safe(true);

            handle
                .write_arrow_batch_reader(reader(), &options)
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
        handle.write_arrow_batch_reader(reader(), &options).unwrap();

        assert_eq!(handle.read_arrow_field(&options).unwrap(), schema());
        assert_eq!(
            handle
                .read_arrow_batch_reader(&options)
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
        handle.write_arrow_batch_reader(reader(), &options).unwrap();

        // The incoming rows declare `id` as text and drop `symbol` entirely. An
        // overwrite replaces rows, so the stored columns survive it and the
        // text is cast back into the stored Int64.
        let loose = DataType::from_fields([DataType::Utf8.required_field("id")])
            .unwrap()
            .required_field("row");
        let incoming = RecordBatch::try_new(
            crate::arrow::schema_from_field(&loose).unwrap(),
            vec![Arc::new(StringArray::from(vec!["7"]))],
        )
        .unwrap();
        handle
            .write_arrow_batch_reader(
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
        let options = handle.record_options().unwrap().with_schema(schema());

        assert_eq!(handle.read_arrow_batch_reader(&options).unwrap().count(), 0);
        assert_eq!(handle.read_arrow_field(&options).unwrap(), schema());
    }

    #[test]
    fn appending_reads_adds_and_rewrites() {
        let mut handle = handle("append.arrows");
        let options = handle.record_options().unwrap().with_schema(schema());

        // Appending to nothing simply writes.
        handle
            .append_arrow_batch_reader(reader(), &options)
            .unwrap();
        assert_eq!(rows(&handle, &options), 2);

        handle
            .append_arrow_batch_reader(reader(), &options)
            .unwrap();
        assert_eq!(rows(&handle, &options), 4);
    }

    #[cfg(feature = "parquet")]
    #[test]
    fn the_three_methods_behave_the_same_way_on_parquet() {
        let mut handle = handle("three.parquet");
        let options = handle.record_options().unwrap().with_schema(schema());

        handle
            .append_arrow_batch_reader(reader(), &options)
            .unwrap();
        handle.write_arrow_batch_reader(reader(), &options).unwrap();
        handle
            .append_arrow_batch_reader(reader(), &options)
            .unwrap();

        assert_eq!(rows(&handle, &options), 4);
    }

    #[test]
    fn appending_casts_incoming_batches_to_the_target_shape() {
        let mut handle = handle("cast-append.arrows");
        let options = handle.record_options().unwrap().with_schema(schema());
        handle.write_arrow_batch_reader(reader(), &options).unwrap();

        // The incoming batch merely fits: `id` is narrower and the columns are
        // the other way round.
        let loose = DataType::from_fields([
            DataType::Utf8.nullable_field("symbol"),
            DataType::Int32.required_field("id"),
        ])
        .unwrap()
        .required_field("row");
        let incoming = RecordBatch::try_new(
            crate::arrow::schema_from_field(&loose).unwrap(),
            vec![
                Arc::new(StringArray::from(vec![Some("MSFT")])),
                Arc::new(arrow_array::Int32Array::from(vec![3])),
            ],
        )
        .unwrap();

        handle
            .append_arrow_batch_reader(
                crate::arrow::batch_reader(incoming.schema(), [incoming]),
                &options,
            )
            .unwrap();

        let batches = handle
            .read_arrow_batch_reader(&options)
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
        let options = handle.record_options().unwrap().with_schema(schema());
        handle.write_arrow_batch_reader(reader(), &options).unwrap();
        let before = handle.as_slice().to_vec();

        // Text that is not a number cannot become the declared Int64, and this
        // write is strict, so the append fails while the batches are being
        // encoded - before anything is published.
        let hostile = DataType::from_fields([DataType::Utf8.required_field("id")])
            .unwrap()
            .required_field("row");
        let incoming = RecordBatch::try_new(
            crate::arrow::schema_from_field(&hostile).unwrap(),
            vec![Arc::new(StringArray::from(vec!["not a number"]))],
        )
        .unwrap();

        let message = handle
            .append_arrow_batch_reader(
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

    /// A declared schema is what a read selects *and* casts to: the columns it
    /// names become the encoding's own projection, and what comes back is the
    /// shape it declares rather than the shape the resource stores.
    mod pushdown {
        use super::{Buffer, DataType, Field, IOBase, IORecordOptions, RecordBatchReader, handle};

        use std::sync::Arc;

        use arrow_array::{Float64Array, Int64Array, RecordBatch, StringArray};

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
                crate::arrow::schema_from_field(&wide()).unwrap(),
                vec![
                    Arc::new(Int64Array::from(vec![1, 2])),
                    Arc::new(StringArray::from(vec![Some("AAPL"), None])),
                    Arc::new(Float64Array::from(vec![1.5, 2.5])),
                    Arc::new(StringArray::from(vec![Some("XNAS"), None])),
                ],
            )
            .unwrap();
            handle
                .write_arrow_batch_reader(
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

                let options = plain.with_schema(narrow());
                let reader = handle
                    .read_arrow_batch_reader(&options)
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
                .read_arrow_batch_reader(&plain.clone().with_schema(wide()))
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
                .read_arrow_batch_reader(&plain.with_schema(invented))
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
            let options = handle.record_options().unwrap().with_schema(reversed);

            let batches = handle
                .read_arrow_batch_reader(&options)
                .unwrap()
                .map(std::result::Result::unwrap)
                .collect::<Vec<_>>();
            assert_eq!(batches[0].schema().field(0).name(), "price");
            assert_eq!(batches[0].schema().field(1).name(), "id");
        }

        #[test]
        fn an_absent_resource_narrows_its_declared_schema_too() {
            let handle = handle("absent.arrows");
            let options = handle.record_options().unwrap().with_schema(narrow());

            let reader = handle.read_arrow_batch_reader(&options).unwrap();
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
        assert_eq!(handle.read_range(600, 5).unwrap(), b"trade");
        assert_eq!(std::fs::read(&path).unwrap()[600..605], *b"trade");

        // Closing releases the pages and leaves a working handle behind.
        handle.close().unwrap();
        assert_eq!(handle.cached_pages(), 0);
        assert_eq!(handle.read_range(600, 5).unwrap(), b"trade");

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
                handle.append(b"first").expect("a writable handle"),
                0,
                "{name}"
            );
            assert_eq!(
                handle.append(b"second").expect("a writable handle"),
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
                handle.read_range(2, 3).expect("a readable handle"),
                b"234",
                "{name}"
            );
            // Asking past the end yields what exists rather than failing.
            assert_eq!(
                handle.read_range(8, 100).expect("a readable handle"),
                b"89",
                "{name}"
            );
            assert!(
                handle
                    .read_range(50, 4)
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

        fn ls(
            &self,
            _recursive: bool,
            _include_private: bool,
        ) -> crate::Result<Vec<crate::generic::Holder>> {
            self.listings.set(self.listings.get() + 1);
            Ok(Vec::new())
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
                .child_by(name)
                .expect("a child")
                .write_all_bytes(b"line\n")
                .expect("a write");
        }
        let mut nested = Folder::new(root.join("deep")).expect("a local container");
        nested.truncate(0).expect("a created container");
        nested
            .child_by("c.log")
            .expect("a child")
            .write_all_bytes(b"line\n")
            .expect("a write");

        folder.clear().expect("a clearable container");
        assert!(root.exists(), "the container still exists after clear");
        assert!(
            folder.ls(true, true).expect("a listing").is_empty(),
            "and is empty"
        );

        // An empty container is removable without recursion.
        folder.remove(false).expect("an empty container removes");
        assert!(!root.exists());

        // A populated one is not.
        folder.truncate(0).expect("a created container");
        folder
            .child_by("a.log")
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
        let schema = crate::arrow::schema_from_field(&field).expect("an Arrow schema");

        let options = crate::generic::RecordOptions::Ipc(crate::ipc::IpcOptions::new());
        let mut media = Ipc::new(Buffer::new());
        media
            .write_arrow_batch_reader(batch_reader(schema, []), &options)
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

        // So is an empty one, and so is a folder that does not exist yet.
        let empty = crate::local::Folder::new(path.join("empty")).unwrap();
        assert!(!empty.is_tabular());
        assert!(!empty.is_atomic());

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
}
