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
//! client's own `Stats`, asserted in `rust/tests/s3/`.

#[path = "support/counting_filesystem.rs"]
mod counting_filesystem;
#[path = "support/excel_package.rs"]
mod excel_package;

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
fn fix_catalog_storage_resolves_each_root_path_once() {
    use yggdryl::local::LocalFolder;
    use yggdryl::{DataType, FixRegistry};

    let path = LocalFolder::temporary()
        .unwrap()
        .path()
        .unwrap()
        .join(format!("yggdryl-fix-root-calls-{}", std::process::id()));
    let mut folder = Counted::new(LocalFolder::new(&path).unwrap());
    let calls = Arc::clone(folder.calls());
    let mut field = DataType::utf8().nullable_field("Symbol");
    field.as_fix_mut().set_tag(55).unwrap();
    let registry = FixRegistry::from_fields([field]).unwrap();
    // Counted measures navigation at this root; child handles own the
    // document reads and writes and are outside this tally. No manifest:
    // a dictionary is one namespace, and what each dialect contributed
    // travels on the field it contributed to.
    // Seven documents and four roots: the store's own field shard, the
    // crate's block on its own shard, its `metadata` group and its `fixmsg`
    // component - a store states the whole row, so the crate's three
    // documents are written beside the store's one - plus the built-in
    // market data type, MsgCat and state vocabularies. The three category roots and `codesets/` are each reached
    // once for pruning. Each intrinsic set adds one document lookup and no
    // root lookup; reading still resolves exactly four roots.
    assert_eq!(
        registry
            .codesets()
            .map(|set| set.name())
            .collect::<Vec<_>>(),
        ["marketdatatypecodeset", "msgcatcodeset", "statecodeset"],
    );
    costs(
        "seven documents, four roots",
        &calls,
        "child_by_path=11",
        || {
            registry.commit(&mut folder).unwrap();
        },
    );
    // And four on the way back: the code sets are read before the fields,
    // because a field naming a set the dictionary does not hold is refused.
    costs("four roots", &calls, "child_by_path=4", || {
        assert_eq!(FixRegistry::from_handle(&folder).unwrap(), registry);
    });
    folder.remove(true).unwrap();
}

/// The bridge's own capture, the same bytes the FIX suite reads it from.
const CAPTURE: &[u8] = include_bytes!("fix/ulbridge.log");

/// Reading a capture as text and then as FIX asks storage for it once.
///
/// The two readers compose - the text reader frames and classifies the lines,
/// the codec reads each framed body into a message - and the composition is
/// where a second decode would hide, because each half is correct on its own
/// while the pair reads the file twice. So the count is taken over both at
/// once: one bounded stream, and the codec never reaching past it.
#[test]
fn a_capture_read_as_text_and_then_as_fix_is_one_decode() {
    use yggdryl::media::RecordOptions;
    use yggdryl::text::{TextOptions, read_text_lines};
    use yggdryl::{FixCodec, FixRegistry, IOMedia, Timezone};

    /// How many lines the capture holds, which is how many rows the text
    /// reader answers.
    const LINES: usize = 144;
    /// How many FIX rows they read as: a row for every message the capture
    /// carries that the codec reads - none for the bridge's own prose, which
    /// carries no message at all, and none for the session traffic and the
    /// documents `DEFAULT_REFUSED_MSGTYPES` keeps out of a live read, which
    /// is what this codec is. What a row costs is the same either way; the
    /// count is here so that a reader knows what was drained. It is 84 where
    /// it was 79 since the parse splits each fill off its report (A12): five
    /// filling reports each bring their execution with them.
    const ROWS: usize = 84;
    /// How many messages the lifecycle walk answers for those rows: a
    /// message arriving under the identity the live one arrived under is the
    /// same message logged at another hop, so it restates that one rather
    /// than joining the chain behind it. This codec reads through a bare
    /// registry, which types almost nothing, so most of the capture's rows
    /// state the same little and collapse onto each other. It is 21 where it
    /// was 16 since the parse splits each fill off its report (A12): the five
    /// executions are each a delivery of their own, `FILLED` ending each
    /// chain, so a later fill under one `ExecID` starts afresh.
    const WALKED: usize = 21;
    // Private intake probes the source location once; avoiding public atomic
    // copy staging removes its two additional bound-location probes.
    /// What one bounded stream over the capture costs, before a message is
    /// built from any of it - through the record dispatcher and through the
    /// text door alike: both ask the one container question, because a
    /// folder or a glob is read leaf by leaf through either, and this capture
    /// is one leaf.
    const DECODE: &str =
        "pstream_bytes=1 url=1 bound_location=1 mtime=1 media_type=1 is_container=1 parent=1";

    let handle = source(CAPTURE, "file:///bridge.log");
    let calls = Arc::clone(handle.calls());
    let mut options = TextOptions::new()
        .try_with_rowheader(yggdryl::ULBRIDGE_ROWHEADER)
        .expect("the bridge's row header compiles")
        .with_timezone(Timezone::UTC);
    options.parse_mimetype = true;
    let options: RecordOptions = options.into();
    let codec = FixCodec::new(Arc::new(FixRegistry::new()));

    costs("the capture read as text alone", &calls, DECODE, || {
        let read: usize = handle
            .read_arrow_reader(&options)
            .expect("a text reader")
            .map(|batch| batch.expect("a batch").num_rows())
            .sum();
        assert_eq!(read, LINES);
    });
    // The same count with the codec on top: the messages are built out of the
    // bytes that stream already returned, so the composition costs the decode
    // and nothing besides it.
    costs(
        "the capture read as text and then as FIX",
        &calls,
        DECODE,
        || {
            let text = handle.read_arrow_reader(&options).expect("a text reader");
            let read: usize = codec
                .parse_text_arrow_reader(text)
                .expect("a FIX reader")
                .map(|batch| batch.expect("a batch").num_rows())
                .sum();
            assert_eq!(read, ROWS);
        },
    );
    costs(
        "the decoded capture composed through the FIX lifecycle",
        &calls,
        DECODE,
        || {
            let RecordOptions::Text(options) = &options else {
                panic!("text options")
            };
            let lines = read_text_lines(&handle, options).expect("a text reader");
            let read = codec
                .lifecycle(codec.parse_text_lines(lines))
                .try_fold(
                    0_usize,
                    |read, message: yggdryl::Result<yggdryl::FixMsg>| message.map(|_| read + 1),
                )
                .expect("a walked message");
            assert_eq!(read, WALKED);
        },
    );
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
fn a_wrapper_asks_the_container_question_rather_than_the_kind() {
    use yggdryl::holder::buffered::{Buffered, BufferedOptions};

    // A leaf answers whether it is a container for free where its kind is a
    // request - a store's `HEAD` - so every wrapper forwards the question
    // rather than deriving it from the kind.
    let buffered = Buffered::new(
        source(&payload(64), "file:///lake/part.bin"),
        BufferedOptions::default(),
    );
    let calls = Arc::clone(buffered.handle().calls());
    costs("through a page cache", &calls, "is_container=1", || {
        assert!(!buffered.is_container());
    });

    let cursor = yggdryl::Cursor::new(source(&payload(64), "file:///lake/part.bin"));
    let calls = Arc::clone(cursor.handle().calls());
    costs("through a cursor", &calls, "is_container=1", || {
        assert!(!cursor.is_container());
    });

    let text = yggdryl::text::Text::new(source(b"AAPL\n", "file:///lake/part.txt"));
    let calls = Arc::clone(text.handle().calls());
    costs("through the text medium", &calls, "is_container=1", || {
        assert!(!text.is_container());
    });
}

#[test]
fn draining_through_a_std_reader_is_one_call_not_one_per_doubling() {
    use std::io::Read;

    let handle = source(&payload(4096), "file:///lake/part.bin");
    let calls = Arc::clone(handle.calls());

    // `read_to_end` grows its buffer by doubling and asks for what fits each
    // time; both readers answer the remainder in one call instead. Each asks
    // first whether the handle is a container, whose positional reads - what
    // `read` answers - are empty while its whole read streams its leaves.
    costs(
        "a reader drained to the end",
        &calls,
        "read_all_bytes=1 is_container=1",
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
    assert_eq!(
        calls.snapshot().to_string(),
        "read_all_bytes=1 is_container=1",
        "a cursor"
    );
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

    // A transfer asks once whether the source is a container, before a byte
    // moves: a container streams its leaves end to end, which no one copy or
    // coding of a value is.
    let source = source(&payload(4096), "file:///lake/part.bin");
    let calls = Arc::clone(source.calls());
    costs(
        "a copy",
        &calls,
        "pstream_bytes=1 bound_location=2 media_type=1 is_container=1",
        || {
            let mut destination = Buffer::new();
            source.copy_into(&mut destination).expect("a copy");
        },
    );
    costs(
        "a compression",
        &calls,
        "pstream_bytes=1 media_type=1 is_container=1",
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
    use yggdryl::fs::{BoundLocation, FileSystem, MemoryFileSystem, located};

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

/// A folder's stream of its files, as the handle wrapping the folder sees it
/// and as the store beneath it does.
#[test]
fn streaming_a_folder_is_one_listing_and_one_open_per_file_it_reads() {
    use counting_filesystem::counted_folder;

    const FILES: usize = 5;
    const LINE: &[u8] = b"AAPL,187.23\n";

    let (filesystem, folder) = counted_folder("logs");
    for file in 0..FILES {
        folder
            .child_by_path(&format!("part-{file}.log"))
            .expect("a child")
            .write_all_bytes(LINE)
            .expect("a write");
    }
    let counted = Counted::new(folder);
    let calls = Arc::clone(counted.calls());

    // The wrapper is asked once: the stream owns the listing and every file
    // it opens, so draining it never comes back through the handle.
    let mut stream = None;
    costs(
        "building a folder's stream",
        &calls,
        "pstream_bytes=1",
        || {
            stream = Some(counted.pstream_bytes(0, 16).expect("a stream"));
        },
    );
    costs("draining a folder's stream", &calls, "none", || {
        let read: usize = stream
            .take()
            .expect("the stream")
            .map(|chunk| chunk.expect("bytes").len())
            .sum();
        assert_eq!(read, FILES * LINE.len());
    });
    costs(
        "a whole read of a folder",
        &calls,
        "read_all_bytes=1",
        || {
            assert_eq!(
                counted.read_all_bytes().expect("a read"),
                LINE.repeat(FILES)
            );
        },
    );

    // Beneath it, building the stream starts the listing and nothing else.
    // Each file then costs the one `file_info` that tells a listed leaf from
    // a container - the `fs` backend answers a listed child's kind by asking
    // the store again - and the one open its bytes are read through.
    let mut stream = None;
    assert_eq!(
        filesystem.costs(|| {
            stream = Some(counted.pstream_bytes(0, 16).expect("a stream"));
        }),
        "list=1",
        "building the stream"
    );
    assert_eq!(
        filesystem.costs(|| {
            stream.take().expect("the stream").for_each(|chunk| {
                chunk.expect("bytes");
            });
        }),
        format!("file_info={FILES} open_input_stream={FILES}"),
        "draining the stream"
    );

    // A file wholly before the position is passed by its size - one more
    // `file_info` - and never opened: two files skipped, three read, the
    // first of them from inside.
    let position = 2 * LINE.len() as u64 + 4;
    assert_eq!(
        filesystem.costs(|| {
            let read: usize = counted
                .pstream_bytes(position, 16)
                .expect("a stream")
                .map(|chunk| chunk.expect("bytes").len())
                .sum();
            assert_eq!(read, (FILES - 2) * LINE.len() - 4);
        }),
        format!(
            "file_info={} list=1 open_input_file=1 open_input_stream={}",
            FILES + 3,
            FILES - 3
        ),
        "a stream from inside the third file"
    );

    // A range is streamed in batches no wider than itself, so a range inside
    // the first file opens that file alone, and an empty range lists nothing.
    assert_eq!(
        filesystem.costs(|| {
            assert_eq!(counted.read_range_bytes(0, 4).expect("a range"), &LINE[..4]);
        }),
        "file_info=1 list=1 open_input_stream=1",
        "a range inside the first file"
    );
    assert_eq!(
        filesystem.costs(|| {
            assert!(counted.read_range_bytes(0, 0).expect("a range").is_empty());
        }),
        "none",
        "an empty range"
    );

    // A range wider than one batch still asks each batch for no more than it
    // owes, so the last batch ending inside the first file opens no second.
    let (filesystem, folder) = counted_folder("wide");
    let wide = payload(100_000);
    folder
        .child_by_path("part-0.log")
        .expect("a child")
        .write_all_bytes(&wide)
        .expect("a write");
    folder
        .child_by_path("part-1.log")
        .expect("a child")
        .write_all_bytes(LINE)
        .expect("a write");
    assert_eq!(
        filesystem.costs(|| {
            assert_eq!(
                folder.read_range_bytes(0, wide.len()).expect("a range"),
                wide
            );
        }),
        "file_info=1 list=1 open_input_stream=1",
        "a range wider than a batch, inside the first file"
    );
}

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

    /// A wrapper already owns its encoding and selection. Omitting the options
    /// must not rediscover the medium or open another stream below that wrapper.
    #[test]
    fn iomedia_omitted_options_cost_the_same_io_as_explicit_owned_options() {
        use yggdryl::holder::counted::{Call, Group};
        use yggdryl::ipc::{Ipc, IpcOptions};
        use yggdryl::media::{IORecordOptions, RecordOptions};
        use yggdryl::{ArrowCastOptions, IOMode, Serie, SerieReader};

        for rows in [64, 4_096] {
            let selected = IpcOptions::new().with_select("symbol").unwrap();
            let raw = written("file:///held.arrows", rows);
            let calls = Arc::clone(raw.calls());
            let source = Ipc::new(raw).with_options(selected.clone());
            calls.reset();
            let options = source.record_options().unwrap();
            assert!(calls.snapshot().is_empty(), "owned options require no IO");
            let read = |options| {
                let reader = source.read_arrow(options).unwrap();
                assert_eq!(reader.field().field_len(), 1);
                assert_eq!(reader.field().fields()[0].name(), "symbol");
                reader.map(|column| column.unwrap().len()).sum::<usize>()
            };
            calls.reset();
            assert_eq!(read(Some(&options)), rows);
            let explicit = calls.snapshot();
            calls.reset();
            assert_eq!(read(None), rows);
            let omitted = calls.snapshot();
            assert_eq!(omitted, explicit, "read: {rows} rows");
            assert_eq!(omitted.get(Call::PstreamBytes), 1);
            assert_eq!(omitted.group(Group::Read), 1, "no extra source read");

            let batch = batch(rows);
            let records = Serie::from_arrow_batch(None, &batch, ArrowCastOptions::new()).unwrap();
            let mut counts = Vec::new();
            let mut bytes = Vec::new();
            for options in [Some(&options), None] {
                let raw = super::source(&[], "file:///held.arrows");
                let calls = Arc::clone(raw.calls());
                let mut target = Ipc::new(raw).with_options(selected.clone());
                let reader = SerieReader::from_serie(records.clone()).unwrap();
                calls.reset();
                target
                    .write_arrow(reader, IOMode::Overwrite, options)
                    .unwrap();
                let tally = calls.snapshot();
                assert_eq!(
                    tally.group(Group::Read),
                    0,
                    "an overwrite reads no stored rows"
                );
                counts.push(tally);
                bytes.push(target.read_all_bytes().unwrap());
                // Plain options prove the selected shape was actually stored.
                let plain = RecordOptions::Ipc(IpcOptions::new());
                let stored = target.read_arrow(Some(&plain)).unwrap();
                assert_eq!(stored.field().field_len(), 1);
                assert_eq!(stored.field().fields()[0].name(), "symbol");
                assert_eq!(
                    stored.map(|column| column.unwrap().len()).sum::<usize>(),
                    rows
                );
            }
            assert_eq!(counts[0], counts[1], "write: {rows} rows");
            assert_eq!(bytes[0], bytes[1], "the selected stream is identical");
        }
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

    /// A Parquet file past a megabyte is read footer first: one read of its
    /// end, which holds the footer, then its column chunks as one range.
    #[cfg(feature = "parquet")]
    #[test]
    fn a_parquet_read_past_a_megabyte_reads_its_footer_first() {
        use yggdryl::media::RecordOptions;
        use yggdryl::parquet::ParquetOptions;

        let media_type = Url::from_str("file:///lake/large.parquet")
            .expect("a location")
            .media_type();
        let mut sink = Buffer::new();
        sink.set_media_type(media_type.clone());
        let written = RecordOptions::Parquet(
            ParquetOptions::new().with_compression(parquet::basic::Compression::UNCOMPRESSED),
        );
        sink.overwrite_arrow_batch(batch(200_000), &written)
            .expect("a write");
        assert!(sink.size() > 1024 * 1024, "{} bytes", sink.size());
        let mut source = Buffer::from_bytes(sink.read_all_bytes().expect("the bytes"));
        source.set_media_type(media_type);
        let handle = Counted::new(source);
        let calls = Arc::clone(handle.calls());
        let options = handle.record_options().expect("record options");

        costs(
            "parquet: a full read past a megabyte",
            &calls,
            "read_range_bytes=2 size=1 media_type=1 is_container=1",
            || {
                let read: usize = handle
                    .read_arrow_reader(&options)
                    .expect("a reader")
                    .map(|batch| batch.expect("a batch").num_rows())
                    .sum();
                assert_eq!(read, 200_000);
            },
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
    fn excel_costs() {
        // A workbook is a ZIP package, and the record doors read it as the
        // core's record doors read every archive: one streamed copy into a
        // buffer of the medium's own, which the two questions about the
        // handle's location (`bound_location`, `parent`) decide cannot be
        // reopened in place. Every dimension is then read off that copy -
        // the schema and the rows once each, the row count a second pass
        // over the same copy, never a per-row call - and the column count
        // asks the handle's size once more to learn nothing new.
        // Private intake probes the source location once; avoiding public atomic
        // copy staging removes its two additional bound-location probes.
        surfaces(
            "excel",
            "file:///lake/part.xlsx",
            "pstream_bytes=1 bound_location=1 media_type=1 is_container=1 parent=1",
            "pstream_bytes=1 bound_location=1 media_type=1 is_container=1 parent=1",
            "pstream_bytes=1 size=1 bound_location=1 media_type=2 is_container=2 parent=1",
            "pstream_bytes=1 bound_location=1 media_type=2 is_container=2 parent=1",
        );
    }

    #[test]
    fn xmla_costs() {
        // A rowset document is held whole, as every structured text document
        // is: XML has no frame to read a prefix of. So the schema, the row
        // count and the rows are each one read of the whole handle, and the
        // column count is the schema's read - never a second one to break a
        // tie, and never a per-row call.
        surfaces(
            "xmla",
            "file:///lake/part.xmla",
            "read_all_bytes=1 media_type=2 is_container=1",
            "read_all_bytes=1 media_type=2 is_container=1",
            "read_all_bytes=1 size=1 media_type=3 is_container=2",
            "read_all_bytes=1 media_type=3 is_container=2",
        );
    }

    #[test]
    fn csv_costs() {
        // A CSV streams, so every surface is one `pstream_bytes` over the
        // handle and never a whole read, and the counts are the plain-text
        // medium's without its `mtime`, because no column dates the rows.
        // The schema and the rows are the same read: the header and the
        // sample are cut from the transport the rows then stream from, and
        // that transport is the owned one - the `url` the rows are located
        // by, the `bound_location` and `parent` asks `owned_handle` makes
        // before it copies a buffer, and the one `media_type` the copy
        // takes over. The column count is the header's width read off a
        // borrowed transport - the `size` is the empty check the dimension
        // defaults make first - and the row count walks the records on the
        // same borrowed transport, reading no cell and asking for no `url`.
        surfaces(
            "csv",
            "file:///lake/part.csv",
            "pstream_bytes=1 url=1 bound_location=3 media_type=1 is_container=1 parent=1",
            "pstream_bytes=1 url=1 bound_location=3 media_type=1 is_container=1 parent=1",
            "pstream_bytes=1 size=1 url=1 media_type=2 is_container=2",
            "pstream_bytes=1 media_type=2 is_container=2",
        );
    }

    #[test]
    fn text_costs() {
        // Plain-text rows are the fifteen event columns and `body`, so this
        // one is read rather than written from a batch. The one `mtime` call
        // per read buys no column of its own any more - it is what dates the
        // rows, and it is a fact about the handle, so every row shares the one
        // answer.
        let media_type = Url::from_str("file:///lake/part.txt")
            .expect("a location")
            .media_type();
        let mut source = Buffer::from_bytes(b"AAPL,187.23\nMSFT,410.10\nGOOG,180.00\n".to_vec());
        source.set_media_type(media_type);
        let handle = Counted::new(source);
        let calls = Arc::clone(handle.calls());
        let options = handle.record_options().expect("record options");

        // Private intake probes the source location once; avoiding public atomic
        // copy staging removes its two additional bound-location probes.
        costs(
            "text: the schema",
            &calls,
            "pstream_bytes=1 url=1 bound_location=1 mtime=1 media_type=1 is_container=1 parent=1",
            || {
                handle.read_arrow_field(&options).expect("a field");
            },
        );
        costs(
            "text: the column count",
            &calls,
            "size=1 media_type=1 is_container=2",
            || {
                assert_eq!(handle.column_size().expect("columns"), 16);
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
            "pstream_bytes=1 url=1 bound_location=1 mtime=1 media_type=1 is_container=1 parent=1",
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
/// counts its own calls instead - `ZipArchive::handle_reads` and `handle_writes`
/// tally every crossing it makes - which is the same measurement taken one
/// layer in, and the one its docs publish.
mod zip {
    use yggdryl::holder::{Buffer, Holder};
    use yggdryl::zip::{ZipArchive, ZipNode};
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
    fn archive(codec: Codec, stride: u64, size: usize) -> ZipNode {
        let root = ZipArchive::new(Holder::buffer(Buffer::new()))
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
    fn stored(name: &str, codec: Codec, stride: u64, size: usize) -> (ZipNode, std::path::PathBuf) {
        let path = std::env::temp_dir().join(format!(
            "yggdryl-calls-zip-{name}-{}.zip",
            std::process::id()
        ));
        let _ = std::fs::remove_file(&path);
        {
            let root = ZipArchive::new(Holder::file(&path).expect("a local archive"))
                .with_restart_stride(stride)
                .mount();
            root.archive()
                .write_member_with("blob.bin", &payload(size), codec)
                .expect("the member writes");
            root.archive().flush().expect("the directory publishes");
        }
        let root = ZipArchive::new(Holder::file(&path).expect("a local archive")).mount();
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
        let root = ZipArchive::new(Holder::buffer(Buffer::new())).mount();
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

    /// Bytes no coding shrinks, so a compressed member spans many batches.
    fn noise(size: usize) -> Vec<u8> {
        let mut state = 0x9E37_79B9_7F4A_7C15_u64;
        (0..size)
            .map(|_| {
                state ^= state << 13;
                state ^= state >> 7;
                state ^= state << 17;
                state as u8
            })
            .collect()
    }

    #[test]
    fn copying_a_member_moves_its_encoded_bytes_a_batch_a_call_and_decodes_nothing() {
        let batch = yggdryl::DEFAULT_STREAM_BATCH_SIZE as u64;
        let source = ZipArchive::new(Holder::buffer(Buffer::new()))
            .with_restart_stride(64 * 1024)
            .mount();
        let original = source
            .archive()
            .write_member_with("noise.bin", &noise(300 * 1024), Codec::Deflate)
            .expect("the member writes");
        let batches = original.compressed_size().div_ceil(batch);
        assert!(batches > 1, "{} encoded bytes", original.compressed_size());

        // The source wrote the member, so it knows where its bytes start:
        // the copy reads the encoded bytes one batch a call and nothing else
        // - no restart proof, no local header, no decoded window - and writes
        // them one batch a call, the header riding the first.
        let target = ZipArchive::new(Holder::buffer(Buffer::new()));
        target.entries().expect("the empty index");
        let (source_reads, target_reads) = (source.archive().handle_reads(), target.handle_reads());
        let copied = target
            .copy_member_from(source.archive(), "noise.bin")
            .expect("the member copies");
        assert_eq!(source.archive().handle_reads() - source_reads, batches);
        assert_eq!(target.handle_reads(), target_reads);
        assert_eq!(target.handle_writes(), batches);
        assert_eq!(
            (copied.crc32(), copied.compressed_size(), copied.restarts()),
            (
                original.crc32(),
                original.compressed_size(),
                original.restarts()
            )
        );

        // A source mounted over bytes it did not write reads the local
        // header that says where they start, once.
        let (stored, path) = stored("copy", Codec::Deflate, 64 * 1024, 4_096);
        stored.archive().entries().expect("the index");
        let before = stored.archive().handle_reads();
        ZipArchive::new(Holder::buffer(Buffer::new()))
            .copy_member_from(stored.archive(), "blob.bin")
            .expect("the member copies");
        assert_eq!(stored.archive().handle_reads() - before, 2);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn a_streamed_member_is_one_call_per_window_and_one_settle() {
        let root = ZipArchive::new(Holder::buffer(Buffer::new())).mount();
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

/// A random read through a decoding handle costs a seek, not a prefix.
///
/// A charset has no seek of its own, so the naive answer is to decode
/// everything before the byte a caller asked for - which turns reading the
/// tail of a file into reading the file, and reading it at ten offsets into
/// reading it ten times. `Transcoded` instead records where a decode may
/// resume, once, and then seeks. The count is what says so: the index costs
/// one bounded stream whatever the payload, and every read after it costs one
/// more stream that starts *at* a resume point rather than at the beginning.
#[test]
fn a_random_read_through_a_charset_seeks_rather_than_re_decoding() {
    use yggdryl::Charset;
    use yggdryl::charset::Transcoded;

    /// Sixteen strides of payload, so a naive read of the tail would walk
    /// fifteen of them and a seeking one walks at most one.
    const ROWS: usize = 16 * 1024;

    let wire = Charset::Cp1252
        .encode(&"symbol,dÃƒÆ’Ã‚Â©sk,price\nAAPL,London,187.23\n".repeat(ROWS))
        .expect("windows-1252 holds it")
        .into_owned();
    let handle = source(&wire, "file:///trades.csv");
    let calls = Arc::clone(handle.calls());
    let decoded = Transcoded::new(handle, Charset::Cp1252);

    // The first question that needs the index pays for it, once: one
    // streamed pass over the whole value, which is also what measures the
    // decoded size.
    calls.reset();
    let size = decoded.size();
    assert_eq!(calls.snapshot().to_string(), "pstream_bytes=1");
    assert!(size > 0);

    // Every read after it is one stream and nothing else, wherever it lands.
    for offset in [0_u64, 7, size / 3, size / 2, size - 64, size - 1] {
        costs(
            &format!("a 32-byte read at {offset}"),
            &calls,
            "pstream_bytes=1",
            || {
                let _ = decoded.read_range_bytes(offset, 32).unwrap();
            },
        );
    }

    // And the bytes are the bytes: a seeking read answers what a whole read
    // answers at the same offset.
    let whole = decoded.read_all_bytes().unwrap();
    assert_eq!(whole.len() as u64, size);
    assert_eq!(
        decoded.read_range_bytes(size - 32, 32).unwrap(),
        whole[whole.len() - 32..]
    );
    for offset in [0_usize, 1, 5_000, whole.len() - 100] {
        let window = decoded.read_range_bytes(offset as u64, 64).unwrap();
        assert_eq!(window, &whole[offset..offset + window.len()], "{offset}");
    }

    // Asking the size again answers from the index rather than walking again.
    costs("a second size", &calls, "none", || {
        assert_eq!(decoded.size(), whole.len() as u64);
    });
}

mod excel {
    //! What a workbook asks of the package it reads and of the handle it
    //! saves into: parsing reads each part once, and a save reads only what
    //! it copies - each member it did not write moved as it is stored, its
    //! encoded bytes a batch a call, nothing inflated - and writes the target
    //! once.

    use super::{Arc, Buffer, Counted, costs};
    use crate::excel_package::{
        content_types, package, rich_package, root_relationships, shared_strings, styles, workbook,
        workbook_relationships, worksheet,
    };
    use yggdryl::excel::Workbook;

    #[test]
    fn named_table_selection_calls_are_constant() {
        use yggdryl::IOMedia;
        use yggdryl::excel::ExcelOptions;
        use yggdryl::media::{IORecordOptions, RecordOptions};

        // Private intake probes the source location once; avoiding public atomic
        // copy staging removes its two additional bound-location probes.
        // Each public door streams the package once. Member reads stay inside
        // that package, independent of selected and unrelated row counts.
        let expected_calls =
            "pstream_bytes=1 bound_location=1 media_type=1 is_container=1 parent=1";
        for (table_rows, unrelated_rows) in [(2, 16), (2, 256), (32, 16), (32, 256)] {
            let bytes = crate::excel_package::named_table_cost_package(table_rows, unrelated_rows);
            let handle = super::source(&bytes, "file:///lake/named.xlsx");
            let calls = Arc::clone(handle.calls());
            let options = RecordOptions::from(ExcelOptions::new().with_table("Names"));
            calls.reset();
            let field = handle.read_arrow_field(&options).expect("named field");
            let field_calls = calls.snapshot().to_string();
            assert_eq!(
                field
                    .fields()
                    .iter()
                    .map(|child| child.name())
                    .collect::<Vec<_>>(),
                ["id", "name"]
            );
            calls.reset();
            let reader = handle.read_arrow_reader(&options).expect("named reader");
            let mut ids = Vec::new();
            for batch in reader {
                let batch = batch.expect("named batch");
                let values = batch
                    .column(0)
                    .as_any()
                    .downcast_ref::<arrow_array::Float64Array>()
                    .unwrap();
                ids.extend(values.iter().map(|value| value.expect("a selected id")));
            }
            let read_calls = calls.snapshot().to_string();
            assert_eq!(ids, (1..=table_rows).map(f64::from).collect::<Vec<_>>());
            eprintln!(
                "named_table_calls: table_rows={table_rows} unrelated_rows={unrelated_rows} field={field_calls} read={read_calls}"
            );
            assert_eq!(field_calls, expected_calls);
            assert_eq!(read_calls, expected_calls);
        }

        // Canonical result-field binding for a declaration is metadata-only even
        // when the selected table does not exist; actual reading validates it.
        let bytes = crate::excel_package::named_table_cost_package(2, 16);
        let handle = super::source(&bytes, "file:///lake/named.xlsx");
        let calls = Arc::clone(handle.calls());
        let declared = yggdryl::DataType::from(
            yggdryl::StructType::from_fields([
                yggdryl::DataType::Float64.required_field("id"),
                yggdryl::DataType::utf8().required_field("name"),
            ])
            .unwrap(),
        )
        .required_field("row");
        let missing =
            RecordOptions::from(ExcelOptions::new().with_table("Absent")).with_field(declared);
        calls.reset();
        assert_eq!(handle.read_arrow_field(&missing).unwrap().field_len(), 2);
        assert_eq!(calls.snapshot().to_string(), "none");
        assert!(handle.read_arrow_reader(&missing).is_err());
    }

    #[test]
    fn excel_cross_sheet_carried_cut_reads_only_hyperlink_relationship_parts() {
        use crate::excel_package;
        use yggdryl::excel::Paste;

        for kind in ["cf", "dv", "x14cf", "x14dv", "x14spark", "hyperlink"] {
            for registrations in [1_u32, 16] {
                for cells in [64_u32, 4_096] {
                    let bytes = excel_package::carried_cost_package(kind, registrations, cells);
                    let mut workbook = Workbook::from_bytes(bytes).unwrap();
                    workbook.parse_all().unwrap();
                    let source = format!("B2:B{}", registrations + 1);
                    let before = workbook.handle_reads();
                    let moved = workbook.paste(
                        ("Data", source.parse().unwrap()),
                        ("Other", "J10".parse().unwrap()),
                        Paste::All,
                        true,
                    );
                    let reads = workbook.handle_reads() - before;
                    eprintln!(
                        "excel_carried_cut: kind={kind}, registrations={registrations}, cells={cells}, reads={reads}"
                    );
                    assert!(moved.is_ok(), "{kind}: {moved:?}");
                    // A new destination relationship part needs two cold reads
                    // each for source .rels and [Content_Types].xml. The plan
                    // shares those bytes across transfer and inverse capture.
                    assert_eq!(
                        reads,
                        if kind == "hyperlink" { 4 } else { 0 },
                        "{kind}: {registrations} registrations, {cells} cells"
                    );
                }
            }
        }
    }

    #[test]
    fn excel_cross_sheet_hyperlink_controls_name_only_the_parts_they_read() {
        use crate::excel_package;
        use yggdryl::excel::Paste;

        for (kind, expected) in [("location", 0), ("hyperlink_existing", 4)] {
            for registrations in [1_u32, 16] {
                let bytes = excel_package::carried_cost_package(kind, registrations, 64);
                let mut workbook = Workbook::from_bytes(bytes).unwrap();
                workbook.parse_all().unwrap();
                let source = format!("B2:B{}", registrations + 1);
                let before = workbook.handle_reads();
                workbook
                    .paste(
                        ("Data", source.parse().unwrap()),
                        ("Other", "J10".parse().unwrap()),
                        Paste::All,
                        true,
                    )
                    .unwrap();
                let reads = workbook.handle_reads() - before;
                eprintln!(
                    "excel_carried_control: kind={kind}, registrations={registrations}, reads={reads}"
                );
                // A location-only hyperlink has no OPC edge. With an
                // existing destination .rels, source and destination each
                // cost one cold header and one body; no new content-type
                // validation is needed.
                assert_eq!(reads, expected, "{kind}: {registrations} registrations");
            }
        }
    }

    #[test]
    fn excel_threaded_cut_reads_each_part_once_independent_of_cells_or_threads() {
        use crate::excel_package;
        use yggdryl::excel::Paste;
        for threads in [1, 16] {
            for cells in [64, 4_096] {
                let mut workbook =
                    Workbook::from_bytes(excel_package::threaded_cost_package(threads, cells))
                        .unwrap();
                workbook.parse_all().unwrap();
                let before = workbook.handle_reads();
                let moved = workbook.paste(
                    ("Data", "B2".parse().unwrap()),
                    ("Data", "F6".parse().unwrap()),
                    Paste::All,
                    true,
                );
                let reads = workbook.handle_reads() - before;
                eprintln!(
                    "excel_threaded_cut: {threads} threads, {cells} cells, {reads} source reads"
                );
                // Sheet .rels, legacy comments, VML, threaded comments and
                // persons: header + body once each. Workbook and root .rels
                // already have cached headers: one body each. Parent/person
                // validation, prospective graph and inverse share these reads.
                assert_eq!(reads, 5 * 2 + 2, "threads={threads}, cells={cells}");
                assert_eq!(moved.unwrap().to_string(), "F6");
                excel_package::assert_cost_thread_moved(&workbook, threads);
            }
        }
    }

    #[test]
    fn excel_note_cut_reads_each_part_once_independent_of_cells_or_notes() {
        use crate::excel_package;
        use yggdryl::excel::Paste;
        for notes in [1, 16] {
            for cells in [64, 4_096] {
                let mut workbook =
                    Workbook::from_bytes(excel_package::note_cost_package(notes, cells)).unwrap();
                workbook.parse_all().unwrap();
                let before = workbook.handle_reads();
                let moved = workbook.paste(
                    ("Data", "B2".parse().unwrap()),
                    ("Data", "F6".parse().unwrap()),
                    Paste::All,
                    true,
                );
                let reads = workbook.handle_reads() - before;
                eprintln!("excel_note_cut: {notes} notes, {cells} cells, {reads} source reads");
                // Sheet .rels, comments and VML each need header + body.
                // Workbook and package-root .rels already have cached headers:
                // one body each. Selection, graph, rewrite and undo share bytes.
                assert_eq!(reads, 3 * 2 + 2, "notes={notes}, cells={cells}");
                assert_eq!(moved.unwrap().to_string(), "F6");
                excel_package::assert_cost_note_moved(&workbook, notes);
            }
        }
    }

    /// Three sheets, the shared strings and the styles, as another writer
    /// laid them out.
    fn book() -> Vec<u8> {
        let sheet = |text: u32| {
            worksheet(&format!(
                "<row r=\"1\"><c r=\"A1\" t=\"s\"><v>{text}</v></c><c r=\"B1\"><v>7</v></c></row>"
            ))
        };
        let (one, two, three) = (sheet(0), sheet(1), sheet(2));
        package(&[
            ("[Content_Types].xml", &content_types(3, true, true)),
            ("_rels/.rels", &root_relationships()),
            (
                "xl/workbook.xml",
                &workbook(&["Sheet1", "Sheet2", "Sheet3"], false),
            ),
            (
                "xl/_rels/workbook.xml.rels",
                &workbook_relationships(3, true, true),
            ),
            ("xl/worksheets/sheet1.xml", &one),
            ("xl/worksheets/sheet2.xml", &two),
            ("xl/worksheets/sheet3.xml", &three),
            ("xl/sharedStrings.xml", &shared_strings(&["a", "b", "c"])),
            ("xl/styles.xml", &styles(&[], &[0])),
        ])
    }

    #[test]
    fn a_workbook_save_copies_what_it_did_not_write_and_writes_its_target_once() {
        let mut opened = Workbook::from_bytes(book()).unwrap();
        // Opening mounts the archive (two reads) and reads three documents,
        // two reads each: the local header, then the member.
        assert_eq!(opened.handle_reads(), 8);
        // Every sheet, the shared strings and the styles: once each.
        opened.parse_all().unwrap();
        assert_eq!(opened.handle_reads(), 8 + 5 * 2);

        // A save that wrote nothing copies each of the nine members as it
        // is stored: one batch of encoded bytes each, and for the one never
        // read - the content types - its local header first. Nothing is
        // inflated, and no document is read to be rewritten.
        let before = opened.handle_reads();
        opened.into_bytes().unwrap();
        assert_eq!(opened.handle_reads() - before, 9 + 1);

        // A save that wrote one sheet copies the seven members it does not
        // rewrite the same way, and reads two documents whole, once each:
        // the workbook part, which it rewrites to ask for a recalculation,
        // and its relationships, to learn whether a calculation chain is
        // related - none is, so they are among the seven copied. The strings
        // and styles it writes against were parsed already.
        opened
            .sheet_mut("Sheet2")
            .unwrap()
            .set_cell("C1".parse().unwrap(), 1.0)
            .unwrap();
        let before = opened.handle_reads();
        opened.into_package().unwrap();
        assert_eq!(opened.handle_reads() - before, 7 + 2);

        // Saved into a handle, the package is one write and no read of it;
        // the workbook then reads the package it wrote, nothing yet.
        let mut target = Counted::new(Buffer::new());
        let calls = Arc::clone(target.calls());
        costs(
            "a workbook saved into a handle",
            &calls,
            "write_all_bytes=1",
            || {
                opened.write_into(&mut target).unwrap();
            },
        );
        assert_eq!(opened.handle_reads(), 0);

        // Adopting the package it wrote, the workbook reads from it: a save
        // after that reads the new source, never the old one.
        opened
            .sheet_mut("Sheet3")
            .unwrap()
            .set_cell("C1".parse().unwrap(), 1.0)
            .unwrap();
        let package = opened.into_package().unwrap();
        // A caller that wrote the package itself hands it back: no call.
        opened.rebase(package).unwrap();
        assert_eq!(opened.handle_reads(), 0);

        // The removal looks for the charts and pivot caches naming the
        // sheet among the members of the package just adopted, so it mounts
        // it - two reads - and finds none to read.
        let before = opened.handle_reads();
        opened.remove_sheet("Sheet3").unwrap();
        assert_eq!(opened.handle_reads() - before, 2);
        // A save after a removal learns what only the removed sheet reached
        // by reading each relationships part once. Every member's local
        // header is read before the member: the content types, the workbook
        // part and the workbook's relationships are read whole once each to
        // be rewritten - the walk takes the relationships from that one read
        // - at two reads apiece; the package's own relationships are read
        // for the walk, then copied as stored (two and one); and the two
        // sheets kept, the shared strings and the styles are copied as
        // stored, two reads apiece.
        let before = opened.handle_reads();
        opened.into_package().unwrap();
        assert_eq!(opened.handle_reads() - before, 3 * 2 + 3 + 4 * 2);
    }

    #[test]
    fn a_save_after_a_patch_writes_the_styles_from_the_model_and_reads_nothing_more() {
        use yggdryl::excel::StylePatch;

        let mut opened = Workbook::from_bytes(book()).unwrap();
        opened.parse_all().unwrap();
        let bold = StylePatch {
            bold: Some(true),
            ..StylePatch::default()
        };
        // Patching reads nothing: the styles were parsed with the sheets.
        let before = opened.handle_reads();
        opened
            .set_style("Sheet2", &["A1:C1".parse().unwrap()], &bold)
            .unwrap();
        assert_eq!(opened.handle_reads(), before);
        // As a save writing one sheet - the six members it neither writes
        // nor rewrites copied as stored, the content types' local header
        // read first as it was never read, the workbook part and its
        // relationships read whole - and the styles written from the table
        // the patch appended to, where they would have been copied: never
        // read again.
        let saved = opened.into_bytes().unwrap();
        assert_eq!(opened.handle_reads() - before, 6 + 1 + 2);
        assert!(
            Workbook::from_bytes(saved)
                .unwrap()
                .cell_style("Sheet2", "C1".parse().unwrap())
                .unwrap()
                .font
                .bold
        );
    }

    /// The theme is a member read on first use and never again; a workbook
    /// with no theme relationship reads nothing for Office's default.
    #[test]
    fn the_theme_is_read_once_on_first_use_and_not_at_all_without_one() {
        let theme = "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\
            <a:theme xmlns:a=\"http://schemas.openxmlformats.org/drawingml/2006/main\" name=\"Custom\">\
            <a:themeElements><a:clrScheme name=\"Custom\">\
            <a:accent1><a:srgbClr val=\"123456\"/></a:accent1>\
            </a:clrScheme></a:themeElements></a:theme>";
        let relationships = "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\
            <Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\">\
            <Relationship Id=\"rId1\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet\" Target=\"worksheets/sheet1.xml\"/>\
            <Relationship Id=\"rId2\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/theme\" Target=\"theme/theme1.xml\"/>\
            </Relationships>";
        let themed = package(&[
            (
                "[Content_Types].xml",
                content_types(1, false, false).as_str(),
            ),
            ("_rels/.rels", root_relationships().as_str()),
            ("xl/workbook.xml", workbook(&["Sheet1"], false).as_str()),
            ("xl/_rels/workbook.xml.rels", relationships),
            ("xl/worksheets/sheet1.xml", worksheet("").as_str()),
            ("xl/theme/theme1.xml", theme),
        ]);
        let opened = Workbook::from_bytes(themed).unwrap();
        // The member's local header, then the member; the second ask is
        // the theme already read.
        let before = opened.handle_reads();
        assert_eq!(opened.theme().unwrap().color(4), Some(0x12_3456));
        assert_eq!(opened.handle_reads() - before, 2);
        opened.theme().unwrap();
        assert_eq!(opened.handle_reads() - before, 2);

        let plain = Workbook::from_bytes(book()).unwrap();
        let before = plain.handle_reads();
        plain.theme().unwrap();
        plain.theme().unwrap();
        assert_eq!(plain.handle_reads(), before);
    }

    /// Rows opened in a sheet move references in the parts beside it, and
    /// each is read once - the check that refuses a band and the rewrite
    /// share the one read - and never again once rewritten: the workbook
    /// holds the rewritten bytes until it saves them.
    #[test]
    fn a_structural_edit_reads_each_part_it_moves_references_in_once() {
        let mut opened = Workbook::from_bytes(rich_package()).unwrap();
        opened.parse_all().unwrap();
        // The sheet's relationships, its comments, drawing, table and
        // notes, then the one chart and the one pivot cache the package
        // holds - each looked at for the sheet's name - at two reads
        // apiece: the local header, then the member. The printer settings
        // and the hyperlink it relates hold no reference and are not read.
        let before = opened.handle_reads();
        opened.insert_rows("Data", 1, 1).unwrap();
        assert_eq!(opened.handle_reads() - before, 7 * 2);
        // Again: the relationships, their header known, and nothing else -
        // every part the first edit rewrote is read from what it wrote.
        let before = opened.handle_reads();
        opened.insert_rows("Data", 1, 1).unwrap();
        assert_eq!(opened.handle_reads() - before, 1);
    }

    /// Cross-sheet references in charts, pivot caches, drawings and table
    /// formulas are indexed once, including the relationships that identify
    /// each table's owning sheet; later edits reuse that index.
    #[test]
    fn a_part_naming_another_sheet_is_read_once_and_not_on_the_next_edit() {
        let mut opened = Workbook::from_bytes(rich_package()).unwrap();
        opened.parse_all().unwrap();
        // `Report` relates nothing. Besides the chart, pivot cache and
        // drawing, table-formula discovery now reads Data's relationships
        // and table once: two additional parts, each a local header and
        // member. These reads establish the table's host before shifting
        // cross-sheet formulas; the next edit reads none of them again.
        let before = opened.handle_reads();
        opened.insert_rows("Report", 5, 1).unwrap();
        assert_eq!(opened.handle_reads() - before, 5 * 2);
        let before = opened.handle_reads();
        opened.insert_rows("Report", 5, 1).unwrap();
        opened.rename_sheet("Report", "Summary").unwrap();
        assert_eq!(opened.handle_reads() - before, 0);
    }

    /// A rewritten carried formula changes no table ownership. The chart,
    /// cache, drawing and table still name Data and retain their source
    /// bytes, so rebuilding their index would reread those unchanged parts.
    #[test]
    fn a_carried_formula_change_keeps_the_referring_index() {
        let mut parts = crate::excel_package::rich_parts();
        let (_, data) = parts
            .iter_mut()
            .find(|(name, _)| *name == "xl/worksheets/sheet1.xml")
            .unwrap();
        let text = std::str::from_utf8(data).unwrap();
        assert_eq!(text.matches("<formula>1</formula>").count(), 1);
        *data = text
            .replace("<formula>1</formula>", "<formula>Report!A1</formula>")
            .into_bytes();
        let parts: Vec<_> = parts
            .iter()
            .map(|(name, bytes)| (*name, bytes.as_slice()))
            .collect();
        let mut opened = Workbook::from_bytes(package(&parts)).unwrap();
        opened.parse_all().unwrap();
        opened.rename_sheet("Report", "Summary").unwrap();
        let before = opened.handle_reads();
        opened.rename_sheet("Summary", "Renamed").unwrap();
        assert_eq!(opened.handle_reads() - before, 0);
        let archive = Arc::new(yggdryl::zip::ZipArchive::new(
            yggdryl::holder::Holder::buffer(Buffer::from_bytes(opened.into_bytes().unwrap())),
        ));
        let data =
            String::from_utf8(archive.read_member("xl/worksheets/sheet1.xml").unwrap()).unwrap();
        assert!(data.contains("<formula>Renamed!A1</formula>"), "{data}");
    }

    /// Two ordinary tables, with the stationary table either beside the
    /// source or on the other worksheet. Their parts have not been read.
    fn table_cut_book(cross_sheet: bool) -> Workbook {
        use crate::excel_package::{NS, R_NS};

        let relation = |id: u32| {
            format!(
                "<Relationship Id=\"rIdTable{id}\" Type=\"{R_NS}/table\" Target=\"../tables/table{id}.xml\"/>"
            )
        };
        let relationships = |entries: String| {
            format!(
                "<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\">{entries}</Relationships>"
            )
        };
        let table = |id, name, range| {
            format!(
                "<table xmlns=\"{NS}\" id=\"{id}\" name=\"{name}\" displayName=\"{name}\" ref=\"{range}\" totalsRowShown=\"0\"><autoFilter ref=\"{range}\"/><tableColumns count=\"2\"><tableColumn id=\"1\" name=\"Key\"/><tableColumn id=\"2\" name=\"Cost\"/></tableColumns></table>"
            )
        };
        let cells = |columns: [&str; 2], row| {
            if row == 2 {
                format!(
                    "<c r=\"{}2\" t=\"inlineStr\"><is><t>Key</t></is></c><c r=\"{}2\" t=\"inlineStr\"><is><t>Cost</t></is></c>",
                    columns[0], columns[1]
                )
            } else {
                format!(
                    "<c r=\"{}{row}\"><v>{row}</v></c><c r=\"{}{row}\"><v>{}</v></c>",
                    columns[0],
                    columns[1],
                    row * 10
                )
            }
        };
        let source_rows: String = (2..=4)
            .map(|row| {
                format!(
                    "<row r=\"{row}\">{}{}</row>",
                    cells(["B", "C"], row),
                    if cross_sheet {
                        String::new()
                    } else {
                        cells(["H", "I"], row)
                    }
                )
            })
            .collect();
        let target_rows: String = if cross_sheet {
            (2..=4)
                .map(|row| format!("<row r=\"{row}\">{}</row>", cells(["H", "I"], row)))
                .collect()
        } else {
            String::new()
        };
        let source_tables = if cross_sheet {
            "<tableParts count=\"1\"><tablePart r:id=\"rIdTable1\"/></tableParts>"
        } else {
            "<tableParts count=\"2\"><tablePart r:id=\"rIdTable1\"/><tablePart r:id=\"rIdTable2\"/></tableParts>"
        };
        let source_sheet = worksheet(&source_rows)
            .replace("</worksheet>", &format!("{source_tables}</worksheet>"));
        let target_sheet = if cross_sheet {
            worksheet(&target_rows).replace(
                "</worksheet>",
                "<tableParts count=\"1\"><tablePart r:id=\"rIdTable2\"/></tableParts></worksheet>",
            )
        } else {
            worksheet(&target_rows)
        };
        let declarations: String = (1..=2).map(|id| format!(
            "<Override PartName=\"/xl/tables/table{id}.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.spreadsheetml.table+xml\"/>"
        )).collect();
        let types =
            content_types(2, false, false).replace("</Types>", &format!("{declarations}</Types>"));
        let source_rels = relationships(if cross_sheet {
            relation(1)
        } else {
            relation(1) + &relation(2)
        });
        let target_rels = relationships(relation(2));
        let source_table = table(1, "Costs", "B2:C4");
        let target_table = table(2, "Taken", "H2:I4");
        let root_rels = root_relationships();
        let book_part = workbook(&["Data", "Other"], false);
        let book_rels = workbook_relationships(2, false, false).replace(
            "</Relationships>",
            &format!("<Relationship Id=\"rIdOpaque\" Type=\"{R_NS}/customXml\" Target=\"../customXml/item1.xml\"/></Relationships>"),
        );
        // An unrelated part has no cell references; the cut must not read
        // its contents while discovering or checking the two table parts.
        let opaque = format!(
            "<opaque xmlns=\"urn:cost-pin\">{}</opaque>",
            "x".repeat(65_536)
        );
        let mut parts = vec![
            ("[Content_Types].xml", types.as_str()),
            ("_rels/.rels", root_rels.as_str()),
            ("xl/workbook.xml", book_part.as_str()),
            ("xl/_rels/workbook.xml.rels", book_rels.as_str()),
            ("xl/worksheets/sheet1.xml", source_sheet.as_str()),
            ("xl/worksheets/sheet2.xml", target_sheet.as_str()),
            ("xl/worksheets/_rels/sheet1.xml.rels", source_rels.as_str()),
            ("xl/tables/table1.xml", source_table.as_str()),
            ("xl/tables/table2.xml", target_table.as_str()),
            ("customXml/item1.xml", opaque.as_str()),
        ];
        if cross_sheet {
            parts.push(("xl/worksheets/_rels/sheet2.xml.rels", target_rels.as_str()));
        }
        let opened = Workbook::from_bytes(package(&parts)).unwrap();
        opened.parse_all().unwrap();
        opened
    }

    /// A failed compound edit pays only for its forward operation. Rollback
    /// consumes parsed cells and retained parts without reading the source again.
    #[test]
    fn excel_failed_batch_rollback_reads_no_source_parts() {
        use yggdryl::excel::Edit;

        let mut plain = Workbook::from_bytes(book()).unwrap();
        plain.parse_all().unwrap();
        plain.style_sheet().unwrap();
        let before = plain.handle_reads();
        let error = plain
            .apply(Edit::Batch(vec![
                Edit::SetEntries {
                    sheet: "Sheet1".into(),
                    entries: vec![("B1".parse().unwrap(), "8".into())],
                },
                Edit::RowHeight {
                    sheet: "Sheet1".into(),
                    start: 0,
                    count: 1,
                    height: Some(-1.0),
                },
            ]))
            .unwrap_err();
        assert!(matches!(error, yggdryl::Error::InvalidRecord { .. }));
        assert_eq!(plain.handle_reads() - before, 0);
        assert!(!plain.is_dirty());
        assert_eq!(
            plain.sheet("Sheet1").unwrap().scalar("B1".parse().unwrap()),
            7.0.into()
        );

        for edit in [
            Edit::InsertRows {
                sheet: "Data".into(),
                at: 1,
                count: 1,
            },
            Edit::RenameSheet {
                name: "Report".into(),
                to: "Summary".into(),
            },
            Edit::RemoveSheet {
                name: "Data".into(),
            },
        ] {
            let label = edit.label();
            let mut forward = Workbook::from_bytes(rich_package()).unwrap();
            let mut refused = Workbook::from_bytes(rich_package()).unwrap();
            forward.parse_all().unwrap();
            refused.parse_all().unwrap();
            forward.style_sheet().unwrap();
            refused.style_sheet().unwrap();
            let revisions: Vec<_> = refused
                .sheet_names()
                .iter()
                .map(|name| refused.sheet(name).unwrap().revision())
                .collect();
            let baseline = forward.handle_reads();
            forward.apply(edit.clone()).unwrap();
            let forward_reads = forward.handle_reads() - baseline;
            if matches!(edit, Edit::InsertRows { .. }) {
                // Same seven parts as the existing structural-edit pin: each
                // local header and member once, with no read in the inverse.
                assert_eq!(forward_reads, 7 * 2);
            }
            let before = refused.handle_reads();
            let error = refused
                .apply(Edit::Batch(vec![
                    edit,
                    Edit::RowHeight {
                        sheet: "Pivot".into(),
                        start: 0,
                        count: 1,
                        height: Some(-1.0),
                    },
                ]))
                .unwrap_err();
            assert!(
                matches!(error, yggdryl::Error::InvalidRecord { .. }),
                "{label}: {error}"
            );
            let total_reads = refused.handle_reads() - before;
            eprintln!(
                "excel_failed_batch {label}: forward {forward_reads}, including rollback {total_reads}"
            );
            assert_eq!(
                total_reads, forward_reads,
                "{label}: rollback reread a source part"
            );
            assert_eq!(refused.sheet_names(), ["Data", "Report", "Pivot"]);
            assert_eq!(
                refused
                    .sheet_names()
                    .iter()
                    .map(|name| refused.sheet(name).unwrap().revision())
                    .collect::<Vec<_>>(),
                revisions
            );
            assert!(!refused.is_dirty(), "{label}");
        }
    }

    #[test]
    fn excel_table_cut_reads_each_part_once_for_check_and_rewrite() {
        use yggdryl::excel::Paste;

        let mut opened = table_cut_book(false);
        let before = opened.handle_reads();
        let moved = opened.paste(
            ("Data", "B2:C4".parse().unwrap()),
            ("Data", "E7".parse().unwrap()),
            Paste::All,
            true,
        );
        // One worksheet relationships part plus two table parts, each
        // read as its local header and member. Preflight and rewriting
        // share these bytes instead of paying a second read for either.
        assert_eq!(opened.handle_reads() - before, 3 * 2);
        assert_eq!(moved.unwrap().to_string(), "E7:F9");
        let data = opened.sheet("Data").unwrap();
        assert_eq!(data.scalar("E8".parse().unwrap()), 3.0.into());
        assert!(data.cell("B3".parse().unwrap()).is_none());
        assert_eq!(data.scalar("H3".parse().unwrap()), 3.0.into());
    }

    #[test]
    fn excel_table_cut_cross_sheet_success_reads_each_part_once() {
        use yggdryl::excel::Paste;

        for warm in [false, true] {
            let mut opened = table_cut_book(true);
            if warm {
                // Populate the referring index and local member-header cache
                // without rewriting these formula-free table payloads.
                opened.rename_sheet("Other", "Warm").unwrap();
                opened.rename_sheet("Warm", "Other").unwrap();
            }
            let before = opened.handle_reads();
            let moved = opened.paste(
                ("Data", "B2:C4".parse().unwrap()),
                ("Other", "E7".parse().unwrap()),
                Paste::All,
                true,
            );
            // The two worksheets' .rels and their two table parts are read
            // once each for preflight, payload edits, and ownership transfer.
            // Cold members cost header + body; warmed headers cost body only.
            assert_eq!(
                opened.handle_reads() - before,
                4 * if warm { 1 } else { 2 },
                "warm={warm}"
            );
            assert_eq!(moved.unwrap().to_string(), "E7:F9");
            assert!(
                opened
                    .sheet("Data")
                    .unwrap()
                    .cell("B3".parse().unwrap())
                    .is_none()
            );
            let other = opened.sheet("Other").unwrap();
            assert_eq!(other.scalar("E8".parse().unwrap()), 3.0.into());
            assert_eq!(other.scalar("H3".parse().unwrap()), 3.0.into());
        }
    }

    #[test]
    fn excel_table_cut_same_sheet_collision_reads_each_part_once() {
        use yggdryl::excel::Paste;

        let mut opened = table_cut_book(false);
        let before = opened.handle_reads();
        let refused = opened.paste(
            ("Data", "B2:C4".parse().unwrap()),
            ("Data", "H2".parse().unwrap()),
            Paste::All,
            true,
        );
        // The same three parts suffice to refuse before any rewrite.
        assert_eq!(opened.handle_reads() - before, 3 * 2);
        assert!(matches!(refused, Err(yggdryl::Error::InvalidRecord { .. })));
        assert!(!opened.is_dirty());
        assert_eq!(
            opened.sheet("Data").unwrap().scalar("B3".parse().unwrap()),
            3.0.into()
        );
    }

    #[test]
    fn excel_table_cut_cross_sheet_collision_reads_each_part_once() {
        use yggdryl::excel::Paste;

        let mut opened = table_cut_book(true);
        let before = opened.handle_reads();
        let refused = opened.paste(
            ("Data", "B2:C4".parse().unwrap()),
            ("Other", "H2".parse().unwrap()),
            Paste::All,
            true,
        );
        // Each worksheet contributes one relationships part, alongside
        // the source and destination table: four parts, two reads each.
        assert_eq!(opened.handle_reads() - before, 4 * 2);
        assert!(matches!(refused, Err(yggdryl::Error::InvalidRecord { .. })));
        assert!(!opened.is_dirty());
        assert_eq!(
            opened.sheet("Data").unwrap().scalar("B3".parse().unwrap()),
            3.0.into()
        );
    }

    #[test]
    fn excel_table_cut_plain_parsed_cells_need_no_more_reads() {
        use yggdryl::excel::Paste;

        let mut opened = Workbook::from_bytes(book()).unwrap();
        opened.parse_all().unwrap();
        let before = opened.handle_reads();
        let moved = opened.paste(
            ("Sheet1", "A1:B1".parse().unwrap()),
            ("Sheet2", "D3".parse().unwrap()),
            Paste::All,
            true,
        );
        // Table collision intake adds no I/O when no sheet owns a table.
        assert_eq!(opened.handle_reads(), before);
        assert_eq!(moved.unwrap().to_string(), "D3:E3");
        assert_eq!(
            opened
                .sheet("Sheet2")
                .unwrap()
                .scalar("D3".parse().unwrap()),
            "a".into()
        );
        assert!(
            opened
                .sheet("Sheet1")
                .unwrap()
                .cell("A1".parse().unwrap())
                .is_none()
        );
    }

    #[test]
    fn a_save_that_appends_a_string_reads_the_table_it_extends_once_more() {
        let mut opened = Workbook::from_bytes(book()).unwrap();
        opened.parse_all().unwrap();
        opened
            .sheet_mut("Sheet2")
            .unwrap()
            .set_cell("C1".parse().unwrap(), "new")
            .unwrap();
        // As a save writing one sheet - the six members it neither writes
        // nor extends copied as stored, the content types' local header
        // read first as it was never read, the workbook part and its
        // relationships read whole - with the shared strings read whole
        // once more where they would have been copied: the parse kept the
        // items, not the bytes, and the table is extended as the package
        // stores it. One call either way, its local header known from the
        // parse; what it adds is the inflate.
        let before = opened.handle_reads();
        let saved = opened.into_bytes().unwrap();
        assert_eq!(opened.handle_reads() - before, 6 + 1 + 2 + 1);
        assert_eq!(
            Workbook::from_bytes(saved)
                .unwrap()
                .sheet("Sheet2")
                .unwrap()
                .scalar("C1".parse().unwrap()),
            yggdryl::Scalar::from("new")
        );
    }
}

mod provider {
    //! What the XML for Analysis provider asks of the store behind a catalog,
    //! per request. A catalog takes a `Holder`, so the count is taken on a
    //! filesystem behind an `FsFolder` rather than on `Counted`; what a
    //! Discover costs is the catalog's listing and, for the columns, each
    //! table's schema. An Execute reopens its table by URL, which an
    //! `FsFolder` has none of that `Holder::from_url` holds, so its cost is
    //! measured by the `media/xmla/service/execute` benchmark and not pinned
    //! here.

    use std::sync::Arc;

    use arrow_array::{Int64Array, RecordBatch, StringArray};
    use yggdryl::holder::Holder;
    use yggdryl::media::RecordOptions;
    use yggdryl::xmla::{
        Catalog, Discover, PropertyList, Request, RequestType, Response, Service, ServiceOptions,
    };
    use yggdryl::{DataType, Field, IOBase, IOMedia, MimeType, StructType};

    use crate::counting_filesystem::{CountingFileSystem, counted_folder};

    fn trades_field() -> Field {
        StructType::from_fields([
            DataType::utf8().required_field("symbol"),
            DataType::Int64.required_field("size"),
        ])
        .map(DataType::from)
        .expect("a valid root")
        .required_field("row")
    }

    fn trades_batch() -> RecordBatch {
        RecordBatch::try_new(
            trades_field().into_arrow_schema().expect("an Arrow schema"),
            vec![
                Arc::new(StringArray::from(vec!["AAPL", "MSFT", "GOOG"])),
                Arc::new(Int64Array::from(vec![100, 250, 75])),
            ],
        )
        .expect("a batch")
    }

    /// A service over a `market` catalog holding one IPC table, `trades`.
    fn ipc_catalog() -> (Arc<CountingFileSystem>, Service) {
        let (filesystem, folder) = counted_folder("market");
        let root = Holder::from(folder);
        let mut leaf = root
            .child_by_path("trades.arrows")
            .expect("the table resolves");
        let batch = trades_batch();
        leaf.overwrite_arrow_reader(
            yggdryl::arrow::batch_reader(batch.schema(), [batch]),
            &RecordOptions::for_mime_type(&MimeType::ARROW_STREAM).expect("IPC options"),
        )
        .expect("the table is written");
        let service =
            Service::new(ServiceOptions::new()).with_catalog(Catalog::new("market", root));
        (filesystem, service)
    }

    /// The calls one Discover of `request_type`, under the `market` catalog,
    /// makes, by name; the answer is checked to be a rowset of `rows` rows.
    fn discover(
        filesystem: &CountingFileSystem,
        service: &Service,
        request_type: RequestType,
        rows: usize,
    ) -> String {
        let message = Request::from(
            Discover::new(request_type.clone())
                .with_properties(PropertyList::new().with("Catalog", "market")),
        )
        .into_bytes()
        .expect("the request encodes");
        let mut answer = Vec::new();
        let costs = filesystem.costs(|| {
            answer = service.handle(&message, Vec::new()).expect("answered");
        });
        let response = Response::from_bytes(&answer, None)
            .unwrap_or_else(|error| panic!("{request_type}: {error}"));
        assert_eq!(
            response.rows().map(yggdryl::Serie::len),
            Some(rows),
            "{request_type}"
        );
        costs
    }

    #[test]
    fn a_discover_over_an_ipc_catalog_costs_its_listing_and_the_columns_a_schema_read() {
        let (filesystem, service) = ipc_catalog();
        let properties = discover(&filesystem, &service, RequestType::DiscoverProperties, 53);
        let catalogs = discover(&filesystem, &service, RequestType::DbschemaCatalogs, 1);
        let cubes = discover(&filesystem, &service, RequestType::MdschemaCubes, 1);
        let tables = discover(&filesystem, &service, RequestType::DbschemaTables, 1);
        let columns = discover(&filesystem, &service, RequestType::DbschemaColumns, 2);
        // Nothing is read for what the provider states about itself, and a
        // catalog row - or the cube row that restates it - costs nothing over
        // this store. The tables are the one
        // listing of the root plus three `file_info` per table - the `fs`
        // backend answers a listed child's kind and modification time by
        // asking the store again - and the columns add one open of the leaf,
        // the schema read, and one more `file_info`, its size; never a read
        // of a row.
        assert_eq!(
            [properties, catalogs, cubes, tables, columns],
            [
                "none",
                "none",
                "none",
                "file_info=3 list=1",
                "file_info=4 list=1 open_input_file=1",
            ],
            "properties, catalogs, cubes, tables, columns"
        );
    }
}

#[test]
fn excel_scoped_name_cut_uses_loaded_names_without_extra_source_reads() {
    use yggdryl::excel::{CellRange, CellRef, Paste, Workbook};

    for shadow in [false, true] {
        for unrelated in [0, 4_096] {
            let bytes =
                excel_package::scoped_name_cost_package(64, "MiXeDRaTe", 4, unrelated, shadow);
            let mut workbook = Workbook::from_bytes(bytes).unwrap();
            workbook.parse_all().unwrap();
            let before = shadow.then(|| excel_package::table_member_map(&workbook));
            let revisions =
                ["Data", "Other", "Third"].map(|name| workbook.sheet(name).unwrap().revision());
            let reads = workbook.handle_reads();
            let moved = workbook.paste(
                (
                    "Data",
                    CellRange::new(CellRef::new(2, 2), CellRef::new(65, 2)),
                ),
                ("Other", CellRef::new(7, 7)),
                Paste::All,
                true,
            );
            assert_eq!(
                workbook.handle_reads() - reads,
                0,
                "shadow={shadow}, unrelated={unrelated}"
            );
            if let Some(before) = before {
                assert!(matches!(moved, Err(yggdryl::Error::Unsupported { .. })));
                assert_eq!(excel_package::table_member_map(&workbook), before);
                assert_eq!(
                    ["Data", "Other", "Third"].map(|name| workbook.sheet(name).unwrap().revision()),
                    revisions
                );
                assert!(!workbook.is_dirty());
            } else {
                assert_eq!(moved.unwrap().to_string(), "H8:H71");
                assert_eq!(
                    workbook
                        .entry_text("Other", CellRef::new(7, 7))
                        .unwrap()
                        .as_deref(),
                    Some("=Data!mixedrate+Data!mixedrate+Data!mixedrate+Data!mixedrate")
                );
            }
        }
    }
}

#[test]
fn iomedia_declared_result_field_binds_without_any_source_calls() {
    use yggdryl::ipc::IpcOptions;
    use yggdryl::media::{IORecordOptions, RecordOptions};
    use yggdryl::{DataType, IOMedia, StructType};

    // Deliberately not an IPC header: every answer is known from the
    // declaration, so even querying the handle's media type is unnecessary.
    let source = Counted::new(Buffer::from_bytes(b"not a record header".to_vec()));
    let calls = Arc::clone(source.calls());
    let field = StructType::from_fields([DataType::Int64.required_field("id")])
        .map(DataType::from)
        .unwrap()
        .required_field("row");
    let declared = RecordOptions::Ipc(IpcOptions::new().with_field(field.clone()));
    costs("declared identity result field", &calls, "none", || {
        assert_eq!(source.read_arrow_field(&declared).unwrap(), field);
    });
    for filter in ["id > 0", "key > 0"] {
        let options = declared
            .clone()
            .with_select("id as key")
            .unwrap()
            .with_filter(filter)
            .unwrap();
        costs("declared projected result field", &calls, "none", || {
            let output = source.read_arrow_field(&options).unwrap();
            assert_eq!(output.fields().len(), 1);
            assert_eq!(output.fields()[0].name(), "key");
        });
    }
    for options in [
        declared.clone().with_select("missing").unwrap(),
        declared.with_filter("missing > 0").unwrap(),
    ] {
        costs("declared result binding refusal", &calls, "none", || {
            let error = source.read_arrow_field(&options).unwrap_err();
            assert!(matches!(error, yggdryl::Error::InvalidRecord { .. }));
            assert!(error.to_string().contains("missing"));
        });
    }
}

#[test]
fn excel_regions_source_calls_do_not_grow_with_height_or_result_count() {
    // Private intake probes the source location once; avoiding public atomic
    // copy staging removes its two additional bound-location probes.
    // The borrowed public door creates its owned workbook handle once.
    // Sheet/table member reads stay inside that opened package; result
    // count and height never cause another source fetch or size probe.
    let expected = "pstream_bytes=1 bound_location=1 media_type=1 parent=1";
    for rows in [64, 4_096] {
        for count in [1, 32] {
            let bytes = excel_package::regions_cost_package(rows, count, true);
            let handle = source(&bytes, "file:///lake/regions.xlsx");
            let calls = Arc::clone(handle.calls());
            calls.reset();
            let regions = yggdryl::excel::regions(&handle, None).unwrap();
            let actual = calls.snapshot().to_string();
            assert_eq!(regions.len(), count as usize);
            assert_eq!(
                regions.first().unwrap().range.start(),
                yggdryl::excel::CellRef::new(0, 0)
            );
            assert_eq!(
                regions.last().unwrap().range.end(),
                yggdryl::excel::CellRef::new(rows + count - 2, 0)
            );
            eprintln!("excel_regions_source_calls: rows={rows} regions={count} {actual}");
            assert_eq!(actual, expected, "rows={rows} regions={count}");
        }
    }
}

#[cfg(feature = "internals")]
#[test]
fn owned_handle_fallback_uses_one_native_stream_without_a_size_probe() {
    use yggdryl::internals::iobase_hierarchy::owned_handle;

    for length in [64, 4096, 2 * 64 * 1024 + 17] {
        let handle = source(&payload(length), "file:///owned.bin");
        let calls = Arc::clone(handle.calls());
        calls.reset();
        let owned = owned_handle(&handle).unwrap();
        assert_eq!(
            calls.snapshot().to_string(),
            "pstream_bytes=1 bound_location=1 media_type=1 parent=1",
            "{length} bytes"
        );
        assert_eq!(owned.read_all_bytes().unwrap(), payload(length));
    }
}

#[test]
fn excel_named_table_rows_conflict_refuses_field_and_reader_before_stream_io() {
    use yggdryl::holder::counted::Call;
    use yggdryl::media::{IORecordOptions, RecordOptions};
    use yggdryl::{DataType, IOMedia, StructType};
    use yggdryl::{RecordHeader, excel::ExcelOptions};

    let handle = source(b"not an XLSX package", "file:///lake/part.xlsx");
    let calls = Arc::clone(handle.calls());
    let field = DataType::from(
        StructType::from_fields([DataType::Float64.required_field("Amount")]).unwrap(),
    )
    .required_field("row");
    let options = RecordOptions::from(
        ExcelOptions::new()
            .with_table("Sales")
            .with_header(RecordHeader::Rows(2)),
    )
    .with_field(field);
    for schema_only in [true, false] {
        calls.reset();
        let error = if schema_only {
            handle.read_arrow_field(&options).unwrap_err().to_string()
        } else {
            handle
                .read_arrow_reader(&options)
                .err()
                .unwrap()
                .to_string()
        };
        assert!(
            error.contains("$.header") && error.contains("Sales"),
            "{error}"
        );
        for read in [
            Call::PstreamBytes,
            Call::Pread,
            Call::ReadAllBytes,
            Call::ReadRangeBytes,
        ] {
            assert_eq!(
                calls.get(read),
                0,
                "read source bytes on schema_only={schema_only}: {}",
                calls.snapshot()
            );
        }
    }
}

#[test]
fn excel_direct_reader_rejects_named_table_rows_before_byte_io() {
    use yggdryl::holder::counted::Call;
    use yggdryl::{DataType, StructType};
    use yggdryl::{
        RecordHeader,
        excel::{ExcelOptions, read_batch_reader},
    };

    let handle = source(b"not an XLSX package", "file:///lake/part.xlsx");
    let calls = Arc::clone(handle.calls());
    let field = DataType::from(
        StructType::from_fields([DataType::Float64.required_field("Amount")]).unwrap(),
    )
    .required_field("row");
    let options = ExcelOptions::new()
        .with_table("Sales")
        .with_header(RecordHeader::Rows(2));
    calls.reset();
    let error = read_batch_reader(&handle, Some(&field), &options)
        .err()
        .unwrap()
        .to_string();
    assert!(
        error.contains("$.header") && error.contains("Sales"),
        "{error}"
    );
    for read in [
        Call::PstreamBytes,
        Call::Pread,
        Call::ReadAllBytes,
        Call::ReadRangeBytes,
    ] {
        assert_eq!(
            calls.get(read),
            0,
            "read source bytes: {}",
            calls.snapshot()
        );
    }
}

#[test]
fn excel_carried_scope_cut_uses_only_loaded_worksheet_frames() {
    use yggdryl::excel::{CellRange, Paste, Workbook};
    for kind in ["protected", "ignored", "watch"] {
        for registrations in [1_u32, 16] {
            for cells in [64_u32, 4_096] {
                let bytes = excel_package::carried_cost_package(kind, registrations, cells);
                let mut workbook = Workbook::from_bytes(bytes).unwrap();
                workbook.parse_all().unwrap();
                let source: CellRange = format!("B2:B{}", registrations + 1).parse().unwrap();
                let target = "J10".parse().unwrap();
                let before = workbook.handle_reads();
                workbook
                    .paste(("Data", source), ("Other", target), Paste::All, true)
                    .unwrap();
                let reads = workbook.handle_reads() - before;
                // Every moved fact is already in the worksheet frame. This
                // operation adds no relationship, content type or package part.
                assert_eq!(
                    reads, 0,
                    "{kind}, {registrations} registrations, {cells} cells"
                );
                excel_package::assert_cost_carried_scope_moved(&workbook, kind, registrations);
            }
        }
    }
}

#[test]
fn excel_infer_worksheet_source_calls_stay_one_stream_at_both_heights() {
    use yggdryl::IOMedia;
    use yggdryl::media::RecordOptions;
    use yggdryl::{RecordHeader, excel::ExcelOptions};

    let expected = "pstream_bytes=1 bound_location=1 media_type=1 is_container=1 parent=1";
    for rows in [64, 4_096] {
        for explicit in [false, true] {
            let bytes = excel_package::regions_cost_package(rows, 1, false);
            let handle = source(&bytes, "file:///lake/infer.xlsx");
            let calls = Arc::clone(handle.calls());
            let mut options = ExcelOptions::new().with_header(RecordHeader::Infer);
            if explicit {
                options = options.with_range(format!("A1:A{rows}").parse().unwrap());
            }
            let options = RecordOptions::from(options);
            calls.reset();
            let field = handle.read_arrow_field(&options).unwrap();
            assert_eq!(field.fields().len(), 1);
            let field_calls = calls.snapshot().to_string();
            calls.reset();
            let returned: usize = handle
                .read_arrow_reader(&options)
                .unwrap()
                .map(|batch| batch.unwrap().num_rows())
                .sum();
            let read_calls = calls.snapshot().to_string();
            assert_eq!(returned, rows as usize);
            eprintln!(
                "excel_infer_source_calls rows={rows} explicit={explicit} field={field_calls} read={read_calls}"
            );
            assert_eq!(field_calls, expected);
            assert_eq!(read_calls, expected);
        }
    }
}

#[test]
fn excel_worksheet_filter_cut_uses_loaded_names_and_frames_only() {
    use yggdryl::excel::{CellRef, Paste, Workbook};
    for mode in ["same", "full", "body", "partial-interior", "partial-header"] {
        let widths: &[u32] = if mode == "partial-header" {
            &[2, 16]
        } else {
            &[1, 16]
        };
        for &columns in widths {
            for cells in [64, 4_096] {
                let bytes = excel_package::worksheet_filter_cost_package(columns, cells);
                let mut book = Workbook::from_bytes(bytes).unwrap();
                book.parse_all().unwrap();
                let (block, destination) = excel_package::worksheet_filter_cut_case(mode, columns);
                let before = book.handle_reads();
                book.paste(
                    ("Data", block),
                    (destination, CellRef::new(9, 9)),
                    Paste::All,
                    true,
                )
                .unwrap();
                assert_eq!(
                    book.handle_reads() - before,
                    0,
                    "{mode}, {columns} criteria, {cells} cells: loaded inline metadata needs no reads"
                );
                excel_package::assert_cost_worksheet_filter_cut(&book, columns, mode);
            }
        }
    }
}

#[test]
fn excel_named_table_write_reads_once_and_publishes_once_at_both_corpus_sizes() {
    use arrow_array::{Int64Array, RecordBatch, StringArray};
    use yggdryl::{DataType, StructType};
    use yggdryl::{
        RecordHeader,
        excel::{ExcelOptions, overwrite_arrow_reader},
    };

    let field = DataType::from(
        StructType::from_fields([
            DataType::Int64.required_field("id"),
            DataType::utf8().required_field("name"),
        ])
        .unwrap(),
    )
    .required_field("row");
    for (table_rows, unrelated_rows) in [(2, 16), (2, 256), (32, 16), (32, 256)] {
        let bytes = excel_package::named_table_cost_package(table_rows, unrelated_rows);
        for (mode, body_rows) in [
            ("shrink", table_rows - 1),
            ("equal", table_rows),
            ("grow", table_rows + 1),
        ] {
            let mut handle = source(&bytes, "file:///lake/named.xlsx");
            let calls = Arc::clone(handle.calls());
            let schema = field.clone().into_arrow_schema().unwrap();
            let input = RecordBatch::try_new(
                schema.clone(),
                vec![
                    Arc::new(Int64Array::from(
                        (1..=body_rows).map(i64::from).collect::<Vec<_>>(),
                    )),
                    Arc::new(StringArray::from(
                        (0..body_rows).map(|_| "replacement").collect::<Vec<_>>(),
                    )),
                ],
            )
            .unwrap();
            let options = ExcelOptions::new()
                .with_table("Names")
                .with_header(RecordHeader::Source);
            // One private package intake; only the completed package reaches
            // the handle. Resizing and unrelated rows add no external I/O.
            calls.reset();
            overwrite_arrow_reader(
                &mut handle,
                yggdryl::arrow::batch_reader(schema, [input]),
                &options,
            )
            .unwrap();
            let observed = calls.snapshot().to_string();
            eprintln!(
                "named_table_write {mode} rows={table_rows} unrelated={unrelated_rows}: {observed}"
            );
            assert_eq!(
                observed,
                "pstream_bytes=1 write_all_bytes=1 size=1 bound_location=1 media_type=1 parent=1"
            );
            let book =
                yggdryl::excel::Workbook::from_bytes(handle.into_inner().into_bytes()).unwrap();
            assert_eq!(
                book.sheet("Data").unwrap().scalar("B2".parse().unwrap()),
                yggdryl::Scalar::from("replacement")
            );
            assert_eq!(
                book.sheet("Data").unwrap().scalar("D2".parse().unwrap()),
                yggdryl::Scalar::from(2024.0)
            );
        }
    }
}

#[test]
fn excel_named_table_write_late_input_refusal_never_publishes() {
    use arrow_array::{Int64Array, RecordBatch, StringArray};
    use yggdryl::excel::{ExcelOptions, overwrite_arrow_reader};
    use yggdryl::{DataType, Error, StructType};

    let bytes = excel_package::named_table_cost_package(2, 256);
    let mut handle = source(&bytes, "file:///lake/named.xlsx");
    let calls = Arc::clone(handle.calls());
    let schema = DataType::from(
        StructType::from_fields([
            DataType::Int64.required_field("id"),
            DataType::utf8().required_field("name"),
        ])
        .unwrap(),
    )
    .required_field("row")
    .into_arrow_schema()
    .unwrap();
    let batch = |values: Vec<i64>| {
        RecordBatch::try_new(
            schema.clone(),
            vec![
                Arc::new(Int64Array::from(values.clone())),
                Arc::new(StringArray::from(vec!["replacement"; values.len()])),
            ],
        )
        .unwrap()
    };
    let too_long = "x".repeat(32_768);
    let last = RecordBatch::try_new(
        schema.clone(),
        vec![
            Arc::new(Int64Array::from(vec![3])),
            Arc::new(StringArray::from(vec![too_long.as_str()])),
        ],
    )
    .unwrap();
    let input = yggdryl::arrow::batch_reader(schema.clone(), [batch(vec![1, 2]), last]);
    calls.reset();
    let error =
        overwrite_arrow_reader(&mut handle, input, &ExcelOptions::new().with_table("Names"))
            .unwrap_err();
    assert!(matches!(error, Error::InvalidRecord { .. }), "{error:?}");
    let observed = calls.snapshot().to_string();
    eprintln!("named_table_write refused: {observed}");
    assert_eq!(
        observed,
        "pstream_bytes=1 size=1 bound_location=1 media_type=1 parent=1"
    );
    assert_eq!(handle.into_inner().into_bytes(), bytes);
}

#[test]
fn excel_named_totals_resize_reads_once_and_publishes_once() {
    use arrow_array::{Int64Array, RecordBatch};
    use yggdryl::excel::{ExcelOptions, overwrite_arrow_reader};
    use yggdryl::{DataType, StructType};

    let field = DataType::from(
        StructType::from_fields([
            DataType::Int64.required_field("year"),
            DataType::Int64.required_field("qty"),
        ])
        .unwrap(),
    )
    .required_field("row");
    for (mode, body_rows) in [("shrink", 1), ("equal", 2), ("grow", 3)] {
        let bytes = excel_package::named_totals_cost_package(256, 1, false, 0);
        let mut handle = source(&bytes, "file:///lake/named-totals.xlsx");
        let calls = Arc::clone(handle.calls());
        let schema = field.clone().into_arrow_schema().unwrap();
        let input = RecordBatch::try_new(
            schema.clone(),
            vec![
                Arc::new(Int64Array::from(
                    (0..body_rows).map(|row| 2030 + row).collect::<Vec<_>>(),
                )),
                Arc::new(Int64Array::from(
                    (0..body_rows).map(|row| 6 + row).collect::<Vec<_>>(),
                )),
            ],
        )
        .unwrap();
        calls.reset();
        overwrite_arrow_reader(
            &mut handle,
            yggdryl::arrow::batch_reader(schema, [input]),
            &ExcelOptions::new().with_table("Quantities"),
        )
        .unwrap();
        let observed = calls.snapshot().to_string();
        eprintln!("named_totals_resize {mode}: {observed}");
        assert_eq!(
            observed,
            "pstream_bytes=1 write_all_bytes=1 size=1 bound_location=1 media_type=1 parent=1"
        );
    }
}

#[test]
fn excel_partial_carried_cut_uses_only_the_loaded_frames() {
    use yggdryl::excel::{Paste, Workbook};
    for kind in ["protected", "ignored", "hyperlink"] {
        for last_row in [64, 4096] {
            for registrations in [1, 16] {
                let bytes =
                    excel_package::partial_carried_cost_package(kind, last_row, registrations);
                let mut workbook = Workbook::from_bytes(bytes).unwrap();
                workbook.parse_all().unwrap();
                let before = workbook.handle_reads();
                let (source, target) =
                    excel_package::partial_carried_cost_edit(kind, registrations);
                workbook
                    .paste(("Data", source), ("Other", target), Paste::All, true)
                    .unwrap();
                assert_eq!(
                    workbook.handle_reads() - before,
                    0,
                    "{kind}, rows={last_row}, registrations={registrations}"
                );
                excel_package::assert_partial_carried_cost_split(
                    &workbook,
                    kind,
                    registrations,
                    last_row,
                );
            }
        }
    }
}

#[cfg(feature = "internals")]
#[test]
fn excel_reference_resolver_uses_loaded_names_and_sparse_sheets_only() {
    use yggdryl::excel::{CellRef, Formula, Workbook};
    use yggdryl::internals::excel_workbook::{References, Resolved};
    for rows in [64, 4096] {
        for names in [64, 4096] {
            let book =
                Workbook::from_bytes(excel_package::reference_resolver_cost_package(rows, names))
                    .unwrap();
            book.parse_all().unwrap();
            let before = book.handle_reads();
            // Resolver construction is included: names are already owned by
            // the workbook and worksheet parts must not be opened a second time.
            let resolver = References::new(&book).unwrap();
            let origin = CellRef::new(0, 0);
            let formula = Formula::from_file("ResolverNameWithMoreThanInlineStorage_0000", origin);
            let Resolved::Name(binding) = resolver.resolve("Data", &formula, origin).unwrap()
            else {
                panic!()
            };
            let Resolved::Range(view) = resolver.resolve_name_reference(&binding).unwrap() else {
                panic!()
            };
            assert_eq!(view.cells().count(), rows as usize);
            assert_eq!(
                book.handle_reads() - before,
                0,
                "{rows} rows, {names} names"
            );
        }
    }
}
