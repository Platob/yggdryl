//! What every derived surface costs in calls to the handle underneath it.
//!
//! `IOBase` is the one boundary a layer crosses to reach storage, and on a
//! store each crossing is a round trip - so the number of them an operation
//! makes is the property this crate is built around, and these are the tests
//! that hold it. Each states its count exactly rather than as a bound: a read
//! that quietly became two calls is the regression worth catching, and so is
//! an operation that stopped talking to storage at all.
//!
//! [`Counted`] is the instrument. It wraps the byte handle, forwards every
//! call unchanged, and tallies it, so the stack built on top of it is measured
//! rather than argued about. The counts here are what a *layer* asks of
//! storage; how many requests a backend then makes of the network is the object
//! client's own `Stats`, asserted in `src/holder/object/tests/accounting.rs`.

use std::sync::Arc;

use yggdryl::holder::Buffer;
use yggdryl::holder::counted::{Calls, Counted};
use yggdryl::{DigestAlgorithm, IOBase, Url};

/// A handle over `bytes`, named so its media type is what `url` says.
fn source(bytes: &[u8], url: &str) -> Counted<Buffer> {
    let url = Url::from_str(url).expect("a location");
    let mut buffer = Buffer::from_bytes(bytes.to_vec());
    buffer.set_media_type(url.media_type());
    Counted::new(buffer)
}

/// A payload of `size` bytes that does not compress to nothing.
fn payload(size: usize) -> Vec<u8> {
    (0..size).map(|index| (index % 251) as u8).collect()
}

/// Run `operation` and assert it cost exactly `expected` in calls.
///
/// The expectation is the tally's own rendering - `read_all_bytes=1`, or
/// `none` - so a failure names the call that appeared as well as the count.
fn costs(what: &str, calls: &Arc<Calls>, expected: &str, operation: impl FnOnce()) {
    calls.reset();
    operation();
    assert_eq!(calls.snapshot().to_string(), expected, "{what}");
}

#[test]
fn a_byte_read_is_one_call_whichever_shape_it_takes() {
    let handle = source(&payload(4096), "file:///lake/part.bin");
    let calls = Arc::clone(handle.calls());

    costs("a whole read", &calls, "read_all_bytes=1", || {
        assert_eq!(handle.read_all_bytes().expect("a read").len(), 4096);
    });
    costs("a ranged read", &calls, "read_range_bytes=1", || {
        assert_eq!(handle.read_range_bytes(0, 16).expect("a read").len(), 16);
    });
    costs("a read of the tail", &calls, "read_range_bytes=1", || {
        assert_eq!(handle.read_range_bytes(4080, 16).expect("a read").len(), 16);
    });
    costs("a positional read", &calls, "pread=1", || {
        let mut window = [0_u8; 16];
        assert_eq!(handle.pread(0, &mut window).expect("a read"), 16);
    });
    costs("an exact positional read", &calls, "pread=1", || {
        let mut window = [0_u8; 16];
        handle.pread_exact(0, &mut window).expect("a read");
    });
    costs("a whole stream drain", &calls, "pstream_bytes=1", || {
        let read: usize = handle
            .pstream_bytes(0, 512)
            .expect("a stream")
            .map(|chunk| chunk.expect("bytes").len())
            .sum();
        assert_eq!(read, 4096);
    });
    costs("a whole digest", &calls, "read_digest=1", || {
        handle.read_digest(DigestAlgorithm::Xxh3).expect("a digest");
    });
    costs("a ranged digest", &calls, "read_range_digest=1", || {
        handle
            .read_range_digest(0, 16, DigestAlgorithm::Xxh3)
            .expect("a digest");
    });
}

#[test]
fn a_metadata_question_is_one_call_and_names_what_it_asks() {
    let handle = source(&payload(4096), "file:///lake/part.bin");
    let calls = Arc::clone(handle.calls());

    costs("a length", &calls, "size=1", || {
        assert_eq!(handle.size(), 4096);
    });
    // Emptiness is a length, and says so rather than reading anything.
    costs("emptiness", &calls, "size=1", || {
        assert!(!handle.is_empty());
    });
    costs("a kind", &calls, "kind=1", || {
        handle.kind();
    });
    costs("a container question", &calls, "is_container=1", || {
        assert!(!handle.is_container());
    });
    costs("an atomicity question", &calls, "is_atomic=1", || {
        handle.is_atomic();
    });
    costs("a tabularity question", &calls, "is_tabular=1", || {
        handle.is_tabular();
    });
    // These two are answers derived from one other answer, and cost that one.
    costs("whether it is I/O", &calls, "is_atomic=1", || {
        handle.is_io();
    });
    costs("a content coding", &calls, "media_type=1", || {
        handle.codec();
    });
}

#[test]
fn draining_through_a_std_reader_is_one_call_not_one_per_doubling() {
    use std::io::Read;

    let handle = source(&payload(4096), "file:///lake/part.bin");
    let calls = Arc::clone(handle.calls());

    // `read_to_end` grows its buffer by doubling and asks for what fits each
    // time; both readers answer the remainder in one call instead.
    costs(
        "a reader drained to the end",
        &calls,
        "read_all_bytes=1",
        || {
            let mut into = Vec::new();
            handle.reader_at(0).read_to_end(&mut into).expect("a read");
            assert_eq!(into.len(), 4096);
        },
    );

    calls.reset();
    let mut into = Vec::new();
    handle.cursor().read_to_end(&mut into).expect("a read");
    assert_eq!(into.len(), 4096);
    assert_eq!(calls.snapshot().to_string(), "read_all_bytes=1", "a cursor");
}

#[test]
fn a_write_is_one_call_and_a_transfer_is_one_stream() {
    let handle = source(b"", "file:///lake/part.bin");
    let calls = Arc::clone(handle.calls());
    let mut handle = handle;

    costs("a whole write", &calls, "write_all_bytes=1", || {
        handle.write_all_bytes(b"AAPL,187.23\n").expect("a write");
    });
    costs("an append", &calls, "append_bytes=1", || {
        handle.append_bytes(b"MSFT,410.10\n").expect("an append");
    });
    costs("a positional write", &calls, "pwrite=1", || {
        handle.pwrite(0, b"GOOG").expect("a write");
    });
    costs("an exact positional write", &calls, "pwrite=1", || {
        handle.pwrite_all(0, b"GOOG").expect("a write");
    });
    costs("a truncation", &calls, "truncate=1", || {
        handle.truncate(4).expect("a truncation");
    });
    costs("a clear", &calls, "clear=1", || {
        handle.clear().expect("a clear");
    });

    let source = source(&payload(4096), "file:///lake/part.bin");
    let calls = Arc::clone(source.calls());
    costs(
        "a copy",
        &calls,
        "pstream_bytes=1 bound_location=2 media_type=1",
        || {
            let mut destination = Buffer::new();
            source.copy_into(&mut destination).expect("a copy");
        },
    );
    costs(
        "a compression",
        &calls,
        "pstream_bytes=1 media_type=1",
        || {
            let mut destination = Buffer::new();
            source
                .compress_into(&mut destination, yggdryl::Codec::Gzip)
                .expect("a compression");
        },
    );
}

#[test]
fn a_coding_reads_the_value_once_however_it_is_asked() {
    let payload = payload(4096);
    let encoded = yggdryl::Codec::Gzip.dump(&payload).expect("an encoding");
    let inner = source(&encoded, "file:///lake/part.bin.gz");
    let calls = Arc::clone(inner.calls());
    let coding = yggdryl::coding::Coding::new(inner, yggdryl::Codec::Gzip);

    // Every one of these would be two passes over the value if the wrapper
    // inherited the `size`-then-read defaults: one to measure, one to take.
    costs("a whole read", &calls, "pstream_bytes=1", || {
        assert_eq!(coding.read_all_bytes().expect("a read"), payload);
    });
    costs("a ranged read", &calls, "pstream_bytes=1", || {
        assert_eq!(coding.read_range_bytes(0, 16).expect("a read").len(), 16);
    });
    costs("a decoded length", &calls, "pstream_bytes=1", || {
        assert_eq!(coding.size(), 4096);
    });
    costs("a stream drain", &calls, "pstream_bytes=1", || {
        for chunk in coding.pstream_bytes(0, 512).expect("a stream") {
            chunk.expect("bytes");
        }
    });
    costs("a digest", &calls, "pstream_bytes=1 kind=1", || {
        coding.read_digest(DigestAlgorithm::Xxh3).expect("a digest");
    });
}

#[test]
fn a_warm_page_cache_asks_the_handle_for_nothing() {
    use yggdryl::holder::buffered::BufferedOptions;

    let inner = source(&payload(4096), "file:///lake/part.bin");
    let calls = Arc::clone(inner.calls());
    let cached = inner.buffered(BufferedOptions::default().with_page_size(1024));

    // The first read fetches the page holding the window, and learns the
    // length while it is there.
    costs("a cold ranged read", &calls, "pread=1 size=1", || {
        assert_eq!(cached.read_range_bytes(0, 16).expect("a read").len(), 16);
    });
    // Every later read inside that page is free - including the length bound,
    // which the inherited default would have re-asked the handle for.
    costs("a warm ranged read", &calls, "none", || {
        assert_eq!(cached.read_range_bytes(0, 16).expect("a read").len(), 16);
    });
    costs("another warm ranged read", &calls, "none", || {
        assert_eq!(cached.read_range_bytes(16, 16).expect("a read").len(), 16);
    });
    costs("a length", &calls, "size=1", || {
        assert_eq!(cached.size(), 4096);
    });
}

/// Listings, globs and partitions over a lake of a hundred files.
#[test]
fn walking_a_lake_is_one_listing_however_many_files_are_in_it() {
    use yggdryl::holder::fs::{BoundLocation, FileSystem, MemoryFileSystem, located};

    const PARTITIONS: usize = 20;
    const PER_PARTITION: usize = 5;

    let filesystem: Arc<dyn FileSystem> = Arc::new(MemoryFileSystem::new());
    for partition in 0..PARTITIONS {
        let directory = format!("/lake/year=2026/month={partition:02}");
        filesystem
            .create_dir(&directory, true)
            .expect("a directory");
        for part in 0..PER_PARTITION {
            let mut sink = filesystem
                .open_output_stream(&format!("{directory}/part-{part}.txt"), None)
                .expect("a stream");
            sink.write(b"AAPL,187.23\n").expect("a write");
            sink.close().expect("a close");
        }
    }

    let lake = located(BoundLocation::new(filesystem, "/lake", None).expect("a location"));
    let counted = Counted::new(lake);
    let calls = Arc::clone(counted.calls());
    let leaves = PARTITIONS * PER_PARTITION;

    costs("a recursive listing", &calls, "ls=1", || {
        let found = counted.ls(true, false).count();
        assert!(found >= leaves, "{found} entries");
    });
    costs("a listing of one level", &calls, "ls=1", || {
        assert_eq!(counted.ls(false, false).count(), 1);
    });
    costs(
        "a glob over the subtree",
        &calls,
        "bound_location=2 ls=1",
        || {
            let found = counted.glob("**/*.txt", false).expect("a glob").count();
            assert_eq!(found, leaves);
        },
    );
    costs("selecting one partition", &calls, "ls=1", || {
        let found = counted
            .children_where(&[("month", "05")], false)
            .expect("a selection")
            .count();
        assert_eq!(found, PER_PARTITION);
    });
    // A partition is read out of the location, so it costs no listing at all.
    costs("reading the partitions", &calls, "url=1", || {
        counted.partitions();
    });
}

#[cfg(feature = "arrow")]
mod records {
    use std::sync::Arc;

    use arrow_array::{Int64Array, RecordBatch, StringArray};
    use arrow_schema::{DataType as ArrowDataType, Field as ArrowField, Schema};
    use yggdryl::holder::Buffer;
    use yggdryl::holder::counted::Counted;
    use yggdryl::{IOBase, IOMedia, Url};

    use super::costs;

    fn batch(rows: usize) -> RecordBatch {
        let schema = Arc::new(Schema::new(vec![
            ArrowField::new("id", ArrowDataType::Int64, false),
            ArrowField::new("symbol", ArrowDataType::Utf8, false),
        ]));
        RecordBatch::try_new(
            schema,
            vec![
                Arc::new(Int64Array::from((0..rows as i64).collect::<Vec<_>>())),
                Arc::new(StringArray::from(
                    (0..rows)
                        .map(|index| format!("SYM{index:04}"))
                        .collect::<Vec<_>>(),
                )),
            ],
        )
        .expect("a batch")
    }

    /// A handle holding `rows` rows written in the encoding `url` names.
    fn written(url: &str, rows: usize) -> Counted<Buffer> {
        let media_type = Url::from_str(url).expect("a location").media_type();
        let mut sink = Buffer::new();
        sink.set_media_type(media_type.clone());
        let options = sink.record_options().expect("record options");
        sink.overwrite_arrow_batch(batch(rows), &options)
            .expect("a write");
        let mut source = Buffer::from_bytes(sink.read_all_bytes().expect("the bytes"));
        source.set_media_type(media_type);
        Counted::new(source)
    }

    /// The counts every record encoding shares, plus the ones that differ.
    ///
    /// `is_container=2` is the shape of the two questions a dimension asks -
    /// the encoding, then the route - and it is two *calls* rather than two
    /// round trips: a leaf answers it from its own role and an unresolved
    /// location keeps what it learned the first time.
    fn surfaces(label: &str, url: &str, field: &str, reader: &str, columns: &str, rows: &str) {
        let handle = written(url, 64);
        let calls = Arc::clone(handle.calls());

        costs(
            &format!("{label}: the encoding"),
            &calls,
            "media_type=1 is_container=1",
            || {
                handle.record_options().expect("record options");
            },
        );
        let options = handle.record_options().expect("record options");
        costs(&format!("{label}: the schema"), &calls, field, || {
            handle.read_arrow_field(&options).expect("a field");
        });
        costs(
            &format!("{label}: the column count"),
            &calls,
            columns,
            || {
                assert_eq!(handle.column_size().expect("columns"), 2);
            },
        );
        costs(&format!("{label}: the row count"), &calls, rows, || {
            assert_eq!(handle.row_size().expect("rows"), 64);
        });
        costs(&format!("{label}: a full read"), &calls, reader, || {
            let read: usize = handle
                .read_arrow_reader(&options)
                .expect("a reader")
                .map(|batch| batch.expect("a batch").num_rows())
                .sum();
            assert_eq!(read, 64);
        });
    }

    #[test]
    fn ipc_costs() {
        surfaces(
            "ipc",
            "file:///lake/part.arrow",
            "pstream_bytes=1 url=1 media_type=1 is_container=1 parent=1",
            "pstream_bytes=1 url=1 media_type=1 is_container=1 parent=1",
            "pstream_bytes=1 size=1 media_type=2 is_container=2",
            // A row count walks the message headers and skips every body, so
            // it is one read per message rather than one transfer of the file.
            "pread=8 size=1 media_type=2 is_container=2",
        );
    }

    #[cfg(feature = "parquet")]
    #[test]
    fn parquet_costs() {
        surfaces(
            "parquet",
            "file:///lake/part.parquet",
            "read_all_bytes=1 size=1 media_type=1 is_container=1",
            "read_all_bytes=1 size=1 media_type=1 is_container=1",
            // Both dimensions come out of the footer: the eight-byte tail and
            // the metadata it points at, and no row is decoded.
            "read_range_bytes=2 size=2 media_type=2 is_container=2",
            "read_range_bytes=2 size=2 media_type=2 is_container=2",
        );
    }

    #[test]
    fn avro_costs() {
        surfaces(
            "avro",
            "file:///lake/part.avro",
            "read_all_bytes=1 media_type=1 is_container=1",
            "read_all_bytes=1 media_type=1 is_container=1",
            // One read for the header, whose fields used to be one read each.
            "pread=1 size=2 media_type=2 is_container=2",
            "pread=1 size=2 media_type=2 is_container=2",
        );
    }

    #[test]
    fn text_costs() {
        // Plain-text rows are `url`, `mtime` and `body`, so this one is read
        // rather than written from a batch. The one `mtime` call per read is
        // the whole cost of the column: it is a fact about the handle, so
        // every row shares the answer.
        let media_type = Url::from_str("file:///lake/part.txt")
            .expect("a location")
            .media_type();
        let mut source = Buffer::from_bytes(b"AAPL,187.23\nMSFT,410.10\nGOOG,180.00\n".to_vec());
        source.set_media_type(media_type);
        let handle = Counted::new(source);
        let calls = Arc::clone(handle.calls());
        let options = handle.record_options().expect("record options");

        costs(
            "text: the schema",
            &calls,
            "pstream_bytes=1 url=1 bound_location=3 mtime=1 media_type=1 is_container=1 parent=1",
            || {
                handle.read_arrow_field(&options).expect("a field");
            },
        );
        costs(
            "text: the column count",
            &calls,
            "size=1 media_type=1 is_container=2",
            || {
                assert_eq!(handle.column_size().expect("columns"), 3);
            },
        );
        costs(
            "text: the row count",
            &calls,
            "pstream_bytes=1 url=1 media_type=2 is_container=2",
            || {
                assert_eq!(handle.row_size().expect("rows"), 3);
            },
        );
        costs(
            "text: a full read",
            &calls,
            "pstream_bytes=1 url=1 bound_location=3 mtime=1 media_type=1 is_container=1 parent=1",
            || {
                let read: usize = handle
                    .read_arrow_reader(&options)
                    .expect("a reader")
                    .map(|batch| batch.expect("a batch").num_rows())
                    .sum();
                assert_eq!(read, 3);
            },
        );
    }
}

#[test]
fn a_read_crossing_pages_is_one_fetch_not_one_per_page() {
    use yggdryl::holder::buffered::BufferedOptions;

    let inner = source(&payload(4096), "file:///lake/part.bin");
    let calls = Arc::clone(inner.calls());
    let cached = inner.buffered(BufferedOptions::default().with_page_size(512));

    // Eight pages, and a miss is one inner read whatever it spans - so a scan
    // that crosses them costs one round trip rather than one each.
    costs(
        "a cold whole read of eight pages",
        &calls,
        "pread=1 size=1",
        || {
            assert_eq!(cached.read_all_bytes().expect("a read").len(), 4096);
        },
    );
    costs("the same read, warm", &calls, "size=1", || {
        assert_eq!(cached.read_all_bytes().expect("a read").len(), 4096);
    });
}

#[test]
fn a_write_to_a_cold_cache_does_not_ask_for_a_length_first() {
    use yggdryl::holder::buffered::BufferedOptions;

    let inner = source(&[], "file:///lake/fresh.bin");
    let calls = Arc::clone(inner.calls());
    let mut cached = inner.buffered(BufferedOptions::default().with_page_size(512));

    // Nothing is cached, so no page needs patching and no length decides
    // anything: the write is the one call it looks like.
    costs(
        "the first write to a fresh handle",
        &calls,
        "pwrite=1",
        || {
            assert_eq!(cached.pwrite(0, b"one").expect("a write"), 3);
        },
    );
    // Once a read has taught the cache a length, a write patches what it
    // holds - still without asking, because the size follows from the write.
    assert_eq!(cached.read_all_bytes().expect("a read"), b"one");
    calls.reset();
    costs("a write against a warm cache", &calls, "pwrite=1", || {
        assert_eq!(cached.pwrite(3, b"two").expect("a write"), 3);
    });
    assert_eq!(cached.read_all_bytes().expect("a read"), b"onetwo");
}

/// What an archive asks of the handle its members live in.
///
/// [`Counted`] cannot be the instrument here: an archive holds a `Holder`, and
/// the enum has no variant for a counted handle to arrive as. The archive
/// counts its own calls instead - `Archive::handle_reads` and `handle_writes`
/// tally every crossing it makes - which is the same measurement taken one
/// layer in, and the one its docs publish.
mod zip {
    use yggdryl::holder::zip::{Archive, Node};
    use yggdryl::holder::{Buffer, Holder};
    use yggdryl::{Codec, IOBase};

    /// A payload long enough to hold several restart strides.
    fn payload(size: usize) -> Vec<u8> {
        b"symbol,price,venue\nAAPL,187.23,XNAS\n"
            .iter()
            .copied()
            .cycle()
            .take(size)
            .collect()
    }

    /// An archive of one member, published, under `codec` and `stride`.
    fn archive(codec: Codec, stride: u64, size: usize) -> Node {
        let root = Archive::new(Holder::buffer(Buffer::new()))
            .with_restart_stride(stride)
            .mount();
        root.archive()
            .write_member_with("blob.bin", &payload(size), codec)
            .expect("the member writes");
        root.archive().flush().expect("the directory publishes");
        root
    }

    /// The same archive, written to a file and mounted again from it.
    ///
    /// A mount's cost is what parsing a directory this archive did not write
    /// asks of the handle, so it has to be a handle whose bytes are already
    /// there rather than one this process filled in.
    fn stored(name: &str, codec: Codec, stride: u64, size: usize) -> (Node, std::path::PathBuf) {
        let path = std::env::temp_dir().join(format!(
            "yggdryl-calls-zip-{name}-{}.zip",
            std::process::id()
        ));
        let _ = std::fs::remove_file(&path);
        {
            let root = Archive::new(Holder::file(&path).expect("a local archive"))
                .with_restart_stride(stride)
                .mount();
            root.archive()
                .write_member_with("blob.bin", &payload(size), codec)
                .expect("the member writes");
            root.archive().flush().expect("the directory publishes");
        }
        let root = Archive::new(Holder::file(&path).expect("a local archive")).mount();
        (root, path)
    }

    #[test]
    fn mounting_an_archive_is_two_reads_and_a_listing_is_none() {
        let (fresh, path) = stored("mount", Codec::Identity, 0, 4_096);

        // The size, then the tail that holds the directory.
        assert_eq!(fresh.ls(true, true).count(), 1);
        assert_eq!(fresh.archive().handle_reads(), 2);

        // Every later listing walks the index the archive already holds.
        let quiet = fresh.archive().handle_reads();
        assert_eq!(fresh.ls(true, true).count(), 1);
        assert_eq!(fresh.glob("*.bin", true).expect("a glob").count(), 1);
        assert_eq!(fresh.archive().handle_reads(), quiet);

        // A member this archive did not write costs one read of the local
        // header that says where its bytes are, once and once only.
        let member = fresh.as_leaf("blob.bin").expect("a member");
        member.read_range_bytes(0, 16).expect("a range");
        assert_eq!(fresh.archive().handle_reads() - quiet, 2);
        let warm = fresh.archive().handle_reads();
        member.read_range_bytes(2_000, 16).expect("a range");
        assert_eq!(fresh.archive().handle_reads() - warm, 1);

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn a_positional_read_of_a_stored_member_is_one_call() {
        let root = archive(Codec::Identity, 0, 4_096);
        let member = root.as_leaf("blob.bin").expect("a member");
        // This archive wrote the member, so it never has to read the local
        // header that would say where the bytes start.
        let before = root.archive().handle_reads();
        assert_eq!(
            member.read_range_bytes(1_000, 16).expect("a range").len(),
            16
        );
        assert_eq!(root.archive().handle_reads() - before, 1);
    }

    #[test]
    fn a_seek_into_a_mapped_member_proves_its_map_once() {
        let root = archive(Codec::Deflate, 4 * 1024, 64 * 1024);
        let member = root.as_leaf("blob.bin").expect("a member");

        // The first seek reads the eight bytes that prove the map, then the
        // encoded window it decodes from.
        let before = root.archive().handle_reads();
        member.read_range_bytes(40_000, 16).expect("a range");
        assert_eq!(root.archive().handle_reads() - before, 2);

        // What the proof settled is the map, so no later seek pays for it.
        let before = root.archive().handle_reads();
        member.read_range_bytes(50_000, 16).expect("a range");
        assert_eq!(root.archive().handle_reads() - before, 1);
    }

    #[test]
    fn a_member_write_is_one_call_and_a_publish_is_two() {
        let root = Archive::new(Holder::buffer(Buffer::new())).mount();
        let before = root.archive().handle_writes();
        root.archive()
            .write_member_with("a.bin", &payload(4_096), Codec::Identity)
            .expect("the member writes");
        root.archive()
            .write_member_with("b.bin", &payload(64), Codec::Identity)
            .expect("the member writes");
        assert_eq!(root.archive().handle_writes() - before, 2);

        // The trailer, then the flush behind it; nothing shortened.
        let before = root.archive().handle_writes();
        root.archive().flush().expect("the directory publishes");
        assert_eq!(root.archive().handle_writes() - before, 2);

        // And a flush with nothing pending is not a write.
        let before = root.archive().handle_writes();
        root.archive().flush().expect("nothing to publish");
        assert_eq!(root.archive().handle_writes(), before);
    }

    #[test]
    fn a_streamed_member_is_one_call_per_window_and_one_settle() {
        let root = Archive::new(Holder::buffer(Buffer::new())).mount();
        let long = payload(3 * 1024 * 1024);

        let before = root.archive().handle_writes();
        root.archive()
            .write_member_from("long.bin", std::io::Cursor::new(&long), Codec::Identity)
            .expect("the member streams in");
        assert_eq!(root.archive().handle_writes() - before, 4);

        // A member whose encoded form fits one window is one write, header
        // and bytes together, because they are one record.
        let before = root.archive().handle_writes();
        root.archive()
            .write_member_from(
                "short.bin",
                std::io::Cursor::new(b"symbol"),
                Codec::Identity,
            )
            .expect("the member streams in");
        assert_eq!(root.archive().handle_writes() - before, 1);
    }
}
