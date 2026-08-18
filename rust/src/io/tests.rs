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

mod read_lines {
    //! Lines stream off any handle, decoded, one line in memory at a time.

    use super::*;

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

    fn collect(handle: &Buffer) -> Vec<String> {
        handle
            .read_lines()
            .unwrap()
            .collect::<crate::Result<Vec<_>>>()
            .unwrap()
    }

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
        let lines: Vec<String> = coded
            .read_lines()
            .unwrap()
            .collect::<crate::Result<Vec<_>>>()
            .unwrap();
        assert_eq!(lines, ["one", "two"]);
    }

    #[test]
    fn the_owned_variant_outlives_the_scope_that_built_the_handle() {
        let lines = {
            let handle = named("scoped.txt", b"kept\nalive\n");
            handle.into_read_lines().unwrap()
        };
        assert_eq!(
            lines.collect::<crate::Result<Vec<_>>>().unwrap(),
            ["kept", "alive"]
        );
    }

    #[test]
    fn invalid_utf8_ends_the_iteration_with_an_error() {
        let handle = named("bad.txt", b"fine\n\xFF\xFE\n");
        let mut lines = handle.read_lines().unwrap();
        assert_eq!(lines.next().unwrap().unwrap(), "fine");
        assert!(lines.next().unwrap().is_err());
        assert!(lines.next().is_none(), "an error ends the iteration");
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
        let records: Vec<String> = handle()
            .read_lines_matching(PATTERN)
            .unwrap()
            .collect::<crate::Result<_>>()
            .unwrap();
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
        let records: Vec<String> = coded
            .into_read_lines_matching(PATTERN)
            .unwrap()
            .collect::<crate::Result<_>>()
            .unwrap();
        assert_eq!(records.len(), 4);
        assert!(records[1].contains("at frame two"));
    }

    #[test]
    fn an_invalid_pattern_is_an_error_up_front() {
        assert!(handle().read_lines_matching("([").is_err());
    }
}

mod arrow_lines {
    //! The Arrow projection of matched line records: a text-line surface,
    //! never a fourth record method.

    use super::*;
    use crate::io::LineRecordOptions;
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

    fn options() -> LineRecordOptions {
        LineRecordOptions::new(PATTERN).unwrap()
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
        let error = LineRecordOptions::new(r"^(?<url>\d+)")
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
        let error = LineRecordOptions::new(r"^(?<level>\d)(?<LEVEL>\d)")
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
        let options = LineRecordOptions::new(
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
        let options = LineRecordOptions::new(
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
        let options =
            LineRecordOptions::new(r"^\d{4}-\d{2}-\d{2} \d{2}:\d{2}:\d{2} \[(?<thread_id>\w+)\]")
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
        let options = || LineRecordOptions::new(r"^(?<level>\w+)").unwrap();

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
        let standalone = crate::io::schema_from_pattern(pattern).unwrap();
        let options = LineRecordOptions::new(pattern).unwrap();
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
        let options =
            LineRecordOptions::new(r"^\d{4}-\d{2}-\d{2} \d{2}:\d{2}:\d{2} \[(?<tag>[^;]+)\]")
                .unwrap();
        let handle = named("brackets.log", b"2024-02-01 10:00:00 [a] rest\nb] tail\n");
        let batch = &batches(handle.read_arrow_lines(&options).unwrap())[0];
        assert_eq!(strings(batch, 6), [Some("2024-02-01 10:00:00 [a]".into())]);
        assert_eq!(strings(batch, 10), [Some("a".into())]);
        assert_eq!(strings(batch, 7), [Some("rest\nb] tail".into())]);
    }

    #[test]
    fn the_timestamp_capture_override_reads_a_named_group() {
        let options = LineRecordOptions::new(r"^\[(?<level>[^\]]+)\] (?<ts>\S+)")
            .unwrap()
            .try_with_timestamp_capture("ts")
            .unwrap();
        let handle = named("alt.log", b"[ee] 2024-02-01T10:00:00.5 boom\n");
        let batch = &batches(handle.read_arrow_lines(&options).unwrap())[0];
        assert_eq!(int64(batch, 4), [Some(FIRST_UNIX + 500_000_000)]);

        // A name the pattern does not capture is rejected when set.
        let error = LineRecordOptions::new(r"^\[(?<level>[^\]]+)\]")
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
