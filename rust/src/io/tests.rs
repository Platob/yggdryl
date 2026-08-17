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
