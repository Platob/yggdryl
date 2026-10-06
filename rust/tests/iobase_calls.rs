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
#[cfg(feature = "s3")]
#[path = "support/server.rs"]
mod server;

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
    // Eight documents and four roots: the store's own field shard, the
    // crate's block on its own shard, its `metadata` group and its `fixmsg`
    // component - a store states the whole row, so the crate's three
    // documents are written beside the store's one - plus the built-in
    // market data kind, market data type, plugin side and state vocabularies. The three category roots and `codesets/` are each reached
    // once for pruning. Each intrinsic set adds one document lookup and no
    // root lookup. The sources catalog, `sources.json` at the root, is
    // one more resolution each way whatever the registry holds: a write
    // publishes it where the registry holds an entry and otherwise reads
    // its digest once to know whether a stale one is there to remove, and
    // a read resolves it because an absent document is no source. So a
    // write resolves thirteen and a read five.
    assert_eq!(
        registry
            .codesets()
            .map(|set| set.name())
            .collect::<Vec<_>>(),
        [
            "marketdatakindcodeset",
            "marketdatatypecodeset",
            "msgpluginsidecodeset",
            "statecodeset"
        ],
    );
    costs(
        "eight documents, four roots, the sources catalog",
        &calls,
        "child_by_path=13",
        || {
            registry.commit(&mut folder).unwrap();
        },
    );
    // And five on the way back: the code sets and the sources catalog are
    // read before the fields, because a field naming a set the dictionary
    // does not hold is refused.
    costs(
        "four roots, the sources catalog",
        &calls,
        "child_by_path=5",
        || {
            assert_eq!(FixRegistry::from_handle(&folder).unwrap(), registry);
        },
    );
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
    /// state the same little and collapse onto each other. It was 21 where
    /// it was 16 since the parse splits each fill off its report (A12): the
    /// five executions are each a delivery of their own, `FILLED` ending
    /// each chain, so a later fill under one `ExecID` starts afresh. It is
    /// 55 since the walk reads its input as already cleaned: the rows whose
    /// type the bare registry does not define, which the walk used to
    /// refuse on its own, are walked as the parse handed them over, and
    /// only the rows that collapse onto a live identity fold.
    const WALKED: usize = 55;
    /// What one bounded stream over the capture costs, before a message is
    /// built from any of it - through the record dispatcher and through the
    /// text door alike: both ask the one container question, because a
    /// folder or a glob is read leaf by leaf through either, and this capture
    /// is one leaf.
    const DECODE: &str =
        "pstream_bytes=1 url=1 bound_location=3 mtime=1 media_type=1 is_container=1 parent=1";

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

    // A move reads the source's kind once, then the copy above, then one
    // removal.
    let mut mover = crate::source(&payload(4096), "file:///lake/part.bin");
    let calls = Arc::clone(mover.calls());
    costs(
        "a move",
        &calls,
        "pstream_bytes=1 remove=1 url=1 bound_location=3 media_type=1 kind=1 is_container=1",
        || {
            let mut destination = Buffer::new();
            mover.move_into(&mut destination).expect("a move");
        },
    );
}

/// A create is one call to the handle beneath, won or lost: the store's own
/// exclusive attempt decides, and nothing is asked first.
#[test]
fn create_bytes_is_one_call_won_or_lost() {
    use yggdryl::local::{LocalFile, LocalFolder};

    let mut root = LocalFolder::temporary().unwrap().path().unwrap();
    root.push(format!("yggdryl-create-calls-{}", std::process::id()));
    let path = root.join("claim.json");
    let mut winner = Counted::new(LocalFile::new(&path).unwrap());
    let calls = Arc::clone(winner.calls());
    costs("a create that lands", &calls, "create_bytes=1", || {
        winner
            .create_bytes(b"{\"v\":1}")
            .expect("nothing at the path");
    });

    let mut loser = Counted::new(LocalFile::new(&path).unwrap());
    let calls = Arc::clone(loser.calls());
    costs("a create that loses", &calls, "create_bytes=1", || {
        assert!(loser.create_bytes(b"{}").unwrap_err().is_conflict());
    });
    drop((winner, loser));
    LocalFolder::new(&root).unwrap().remove(true).unwrap();
}

/// What a move between two objects of one store costs in requests, and the
/// two refusals that cost the open alone.
#[cfg(feature = "s3")]
mod object_store {
    use yggdryl::IOBase;
    use yggdryl::holder::Holder;
    use yggdryl::s3::{Credentials, S3Options};

    use crate::server::FakeS3;

    fn file(store: &FakeS3, key: &str) -> Holder {
        let options = S3Options::default()
            .with_environment(false)
            .with_endpoint(store.endpoint())
            .with_region("us-east-1")
            .with_path_style(true)
            .with_credentials(Credentials::new("AKIAIOSFODNN7EXAMPLE", "wJalrXUtnFEMI"));
        Holder::S3File(yggdryl::s3::file_with(&format!("s3://trades/{key}"), options).unwrap())
    }

    fn methods(store: &FakeS3) -> Vec<String> {
        store
            .requests()
            .iter()
            .map(|request| request.method.clone())
            .collect()
    }

    /// A create is one conditioned `PUT` whether it lands or loses: nothing
    /// is asked first, and a lost race is answered by the store's `412`.
    #[test]
    fn create_bytes_is_one_put_won_or_lost() {
        let store = FakeS3::start();
        store.create_bucket("trades");
        let mut winner = file(&store, "lake/claim.json");
        store.clear_requests();
        winner.create_bytes(b"{\"v\":1}").unwrap();
        assert_eq!(methods(&store), ["PUT"]);

        let mut loser = file(&store, "lake/claim.json");
        store.clear_requests();
        assert!(loser.create_bytes(b"{}").unwrap_err().is_conflict());
        assert_eq!(methods(&store), ["PUT"]);
        assert_eq!(store.requests()[0].status, 412);
        assert_eq!(
            store.get("trades", "lake/claim.json").as_deref(),
            Some(&b"{\"v\":1}"[..])
        );
    }

    #[test]
    fn a_move_between_two_objects_is_the_open_the_copy_and_the_removal() {
        let store = FakeS3::start();
        store.create_bucket("trades");
        store.put("trades", "lake/source.bin", b"AAPL,1");
        let mut source = file(&store, "lake/source.bin");
        let mut target = file(&store, "lake/target.bin");
        store.clear_requests();

        // One `HEAD` reads the source's kind; the copy reads it, reads the
        // target it would restore and publishes; one `DELETE` removes the
        // source.
        assert_eq!(source.move_into(&mut target).unwrap(), 6);
        assert_eq!(methods(&store), ["HEAD", "GET", "GET", "PUT", "DELETE"]);
        assert_eq!(
            store.get("trades", "lake/target.bin").as_deref(),
            Some(&b"AAPL,1"[..])
        );
        assert_eq!(store.get("trades", "lake/source.bin"), None);

        // Onto its own location: the kind read, then the refusal by that
        // location, and the object untouched.
        let mut source = file(&store, "lake/target.bin");
        let mut same = file(&store, "lake/target.bin");
        store.clear_requests();
        let error = source.move_into(&mut same).unwrap_err();
        assert!(matches!(error, yggdryl::Error::Conflict { .. }), "{error}");
        assert!(error.to_string().contains("lake/target.bin"), "{error}");
        assert_eq!(methods(&store), ["HEAD"]);
        assert_eq!(
            store.get("trades", "lake/target.bin").as_deref(),
            Some(&b"AAPL,1"[..])
        );

        // A store that refuses the `HEAD` reads as an unknown kind, which one
        // bounded `GET` confirms: refused too, the move fails with the store's
        // own reason, never as absence, and nothing is moved or removed.
        let mut other = file(&store, "lake/other.bin");
        store.clear_requests();
        store.fail_next(403, "AccessDenied", 2);
        let error = source.move_into(&mut other).unwrap_err();
        assert!(!error.is_absent(), "{error}");
        assert_eq!(methods(&store), ["HEAD", "GET"]);
        assert_eq!(
            store.get("trades", "lake/target.bin").as_deref(),
            Some(&b"AAPL,1"[..])
        );
        assert_eq!(store.get("trades", "lake/other.bin"), None);
    }
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

    /// `rows` quotes of a venue and a price, the venue running through
    /// three in key order, changing at a third and at two thirds of the rows.
    fn quotes(rows: usize) -> RecordBatch {
        let schema = Arc::new(Schema::new(vec![
            ArrowField::new("venue", ArrowDataType::Utf8, false),
            ArrowField::new("price", ArrowDataType::Int64, false),
        ]));
        RecordBatch::try_new(
            schema,
            vec![
                Arc::new(StringArray::from(
                    (0..rows)
                        .map(|index| ["XNAS", "XNYS", "XPAR"][index * 3 / rows])
                        .collect::<Vec<_>>(),
                )),
                Arc::new(Int64Array::from((0..rows as i64).collect::<Vec<_>>())),
            ],
        )
        .expect("a batch")
    }

    /// A handle holding `rows` rows written in the encoding `url` names.
    fn written(url: &str, rows: usize) -> Counted<Buffer> {
        written_batch(url, batch(rows))
    }

    /// A handle holding `batch` written in the encoding `url` names.
    fn written_batch(url: &str, batch: RecordBatch) -> Counted<Buffer> {
        let media_type = Url::from_str(url).expect("a location").media_type();
        let mut sink = Buffer::new();
        sink.set_media_type(media_type.clone());
        let options = sink.record_options().expect("record options");
        sink.overwrite_arrow_batch(batch, &options)
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

    /// A stream's windows pull the stream they are cut from and ask the
    /// handle for nothing of their own: a read windowed by its venue, sorted,
    /// each window read before the next is taken, costs the calls the read
    /// drained alone costs.
    #[test]
    fn windowing_a_read_adds_no_call() {
        let handle = written_batch("file:///lake/quotes.arrow", quotes(64));
        let calls = Arc::clone(handle.calls());
        let read = "pstream_bytes=1 url=1 media_type=3 is_container=2 parent=1";
        costs("ipc: a read drained", &calls, read, || {
            let rows: usize = handle
                .read_serie(None)
                .expect("a reader")
                .map(|batch| batch.expect("a batch").len())
                .sum();
            assert_eq!(rows, 64);
        });
        costs("ipc: a read windowed by venue", &calls, read, || {
            let mut windows = 0;
            let mut rows = 0;
            for window in handle
                .read_serie(None)
                .expect("a reader")
                .window_by("venue", true)
                .expect("windows")
            {
                windows += 1;
                for piece in window.expect("a window") {
                    rows += piece.expect("a piece").len();
                }
            }
            assert_eq!((windows, rows), (3, 64));
        });
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
        surfaces(
            "excel",
            "file:///lake/part.xlsx",
            "pstream_bytes=1 bound_location=3 media_type=1 is_container=1 parent=1",
            "pstream_bytes=1 bound_location=3 media_type=1 is_container=1 parent=1",
            "pstream_bytes=1 size=1 bound_location=3 media_type=2 is_container=2 parent=1",
            "pstream_bytes=1 bound_location=3 media_type=2 is_container=2 parent=1",
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

/// A whole write through the cache is the handle's own whole write: one call
/// beneath, every page dropped before it, and the length it wrote kept, so
/// the read after it - one page the write filled exactly - is the fetch alone
/// and asks for no length.
#[test]
fn a_whole_write_through_the_cache_is_the_handles_own() {
    use yggdryl::holder::buffered::BufferedOptions;

    let inner = source(b"stale value", "file:///lake/whole.bin");
    let calls = Arc::clone(inner.calls());
    let mut cached = inner.buffered(BufferedOptions::default().with_page_size(512));
    assert_eq!(cached.read_all_bytes().expect("a read"), b"stale value");

    let fresh = payload(512);
    costs(
        "a whole write through the cache",
        &calls,
        "write_all_bytes=1",
        || {
            cached.write_all_bytes(&fresh).expect("a write");
        },
    );
    let mut probe = vec![0_u8; 512];
    costs("the read after it", &calls, "pread=1", || {
        assert_eq!(cached.pread(0, &mut probe).expect("a read"), 512);
    });
    assert_eq!(probe, fresh);
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
        .encode(&"symbol,désk,price\nAAPL,London,187.23\n".repeat(ROWS))
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
        Discover, PropertyList, Request, RequestType, Response, Service, ServiceOptions,
    };
    use yggdryl::{DataType, Field, FolderCatalog, IOBase, IOMedia, MimeType, StructType};

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
            Service::new(ServiceOptions::new()).with_catalog(FolderCatalog::bound("market", root));
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
        // this store. The tables are the one listing of the root plus two
        // `file_info` per table - the `fs` backend answers a listed child's
        // kind and modification time by asking the store again, each once,
        // since the folder catalog hands the leaf on as the listing gave it -
        // and the columns add one open of the leaf's stream and the schema
        // read; never a read of a row. Moving the provider onto the folder
        // catalog took one `file_info` off a table row and two off a column
        // read: the old catalog asked a leaf's kind three times.
        assert_eq!(
            [properties, catalogs, cubes, tables, columns],
            [
                "none",
                "none",
                "none",
                "file_info=2 list=1",
                "file_info=2 list=1 open_input_stream=1",
            ],
            "properties, catalogs, cubes, tables, columns"
        );
    }
}

mod isin_registry {
    use std::sync::Arc;

    use yggdryl::holder::Buffer;
    use yggdryl::holder::counted::Counted;
    use yggdryl::media::IORecordOptions;
    use yggdryl::{IOBase, IOMedia, IOMode, IdType, Isin, IsinEntry, IsinRegistry, MimeType};

    use super::costs;

    /// A registry is read from a holder in exactly the calls one record read
    /// of it makes: the encoding, then the stream, and nothing of its own.
    #[test]
    fn an_isin_registry_reads_a_holder_in_the_calls_of_one_record_read() {
        let mut registry = IsinRegistry::new();
        registry
            .merge(
                IsinEntry::new(Isin::new("CH0012214059").unwrap())
                    .try_with_code(IdType::Ric, "HOLN.S")
                    .unwrap(),
            )
            .unwrap();
        let mut sink = Buffer::new().with_media_type(MimeType::ARROW_STREAM.into());
        let options = sink
            .record_options()
            .unwrap()
            .with_field(IsinEntry::field());
        sink.write_arrow_reader(
            registry.into_arrow_reader().unwrap(),
            IOMode::Overwrite,
            &options,
        )
        .unwrap();
        let mut source = Buffer::from_bytes(sink.read_all_bytes().unwrap());
        source.set_media_type(MimeType::ARROW_STREAM.into());
        let handle = Counted::new(source);
        let calls = Arc::clone(handle.calls());
        calls.reset();
        let options = handle.record_options().unwrap();
        for batch in handle.read_arrow_reader(&options).unwrap() {
            batch.unwrap();
        }
        let one_read = calls.snapshot().to_string();
        costs("an isin registry read", &calls, &one_read, || {
            let mut registry = IsinRegistry::new();
            assert_eq!(registry.extend_from_handle(&handle).unwrap(), 1);
            assert_eq!(registry.len(), 1);
        });
    }
}

/// What the warehouse costs the store: describing and resolving registered
/// objects nothing, a folder namespace's children one listing plus one
/// listing of `metadata/` per folder entry, a path one listing per level it
/// descends, and a read through `Holder::Table` exactly what the same read
/// on the table's own handle costs.
mod warehouse {
    use yggdryl::holder::Holder;
    use yggdryl::media::RecordOptions;
    use yggdryl::{
        Catalog, FolderCatalog, IOBase, IOMedia, MediaTable, MimeType, NamespaceValue, Table,
        TableValue, Url, Warehouse,
    };

    use crate::counting_filesystem::{CountingFileSystem, counted_folder};

    /// A counted store laid out as a catalog: a root leaf, a namespace of a
    /// leaf and a folder table, and an empty namespace.
    fn store() -> (std::sync::Arc<CountingFileSystem>, yggdryl::fs::FsFolder) {
        let (filesystem, folder) = counted_folder("market");
        for leaf in ["trades.csv", "eu/fills.csv", "eu/lake/part-0.csv"] {
            folder
                .child_by_path(leaf)
                .expect("a child")
                .write_all_bytes(b"symbol,price\nAAPL,187.5\n")
                .expect("written");
        }
        folder
            .child_by_path("empty/.keep")
            .expect("a child")
            .write_all_bytes(b"")
            .expect("written");
        (filesystem, folder)
    }

    #[test]
    fn describing_registering_and_resolving_registered_objects_touch_nothing() {
        let (filesystem, folder) = store();
        let mut warehouse = Warehouse::new();
        let registered = filesystem.costs(|| {
            warehouse
                .register(FolderCatalog::bound("market", Holder::FsFolder(folder)))
                .expect("registered");
            warehouse
                .register(Table::from(
                    MediaTable::new(
                        "lake.eu.trades",
                        Url::from_str("file:///lake/eu/trades.csv").expect("a URL"),
                    )
                    .expect("a table"),
                ))
                .expect("registered");
        });
        let resolved = filesystem.costs(|| {
            warehouse.get("market").expect("the catalog");
            warehouse.table("lake.eu.trades").expect("the table");
            warehouse.namespace("lake.eu").expect("the namespace");
            assert!(
                warehouse
                    .properties_for(&Url::from_str("file:///lake/eu/trades.csv").expect("a URL"))
                    .is_empty()
            );
        });
        assert_eq!([registered, resolved], ["none", "none"]);
    }

    #[test]
    fn a_folder_namespaces_children_cost_one_listing_and_one_per_folder_entry() {
        let (filesystem, folder) = store();
        let catalog = Catalog::from(FolderCatalog::bound("market", Holder::FsFolder(folder)));
        let root = filesystem.costs(|| {
            // Two folders and a leaf: the root listing, then for each folder
            // entry its kind and the listing of its `metadata/`, the one
            // question that tells a table format from a plain folder.
            assert_eq!(catalog.children().count(), 3);
        });
        let eu = catalog.namespace("eu").expect("the namespace");
        let under = filesystem.costs(|| {
            assert_eq!(eu.children().count(), 2);
        });
        let first = filesystem.costs(|| {
            // Taking the first child lists once and classifies one entry.
            assert!(catalog.children().next().is_some());
        });
        // The lists are the contract; the `file_info` is the `fs` backend's
        // own answer to a listed leaf's role, asked once per leaf.
        assert_eq!(
            [root, under, first],
            ["file_info=1 list=3", "file_info=1 list=2", "list=2"],
            "root, under a namespace, first child"
        );
    }

    #[test]
    fn a_path_costs_one_listing_per_level_it_descends() {
        let (filesystem, folder) = store();
        let catalog = Catalog::from(FolderCatalog::bound("market", Holder::FsFolder(folder)));
        let one = filesystem.costs(|| {
            catalog.table("trades").expect("the root table");
        });
        let two = filesystem.costs(|| {
            catalog.table("eu.fills").expect("the table under eu");
        });
        let absent = filesystem.costs(|| {
            assert!(catalog.table("eu.nowhere").expect_err("absent").is_absent());
        });
        // One listing per level, one listing of `metadata/` per folder level
        // passed through, and the matched leaf's role asked once.
        assert_eq!(
            [one, two, absent],
            ["file_info=1 list=1", "file_info=1 list=3", "list=3"],
            "one level, two levels, two levels to an absence"
        );
    }

    #[test]
    fn a_read_through_a_table_handle_costs_what_the_tables_own_handle_costs() {
        let (filesystem, folder) = store();
        let catalog = Catalog::from(FolderCatalog::bound(
            "market",
            Holder::FsFolder(folder.clone()),
        ));
        let options = RecordOptions::for_mime_type(&MimeType::CSV).expect("CSV options");
        let leaf = || folder.child_by_path("eu/fills.csv").expect("the leaf");
        // The leaf as a listing hands it out, which is what the table holds:
        // a listed leaf already carries what the listing said of it.
        let listed = || {
            folder
                .child_by_path("eu")
                .expect("eu")
                .ls(false, false)
                .map(|entry| entry.expect("an entry"))
                .find(|entry| entry.url().and_then(Url::file_name) == Some("fills.csv"))
                .expect("the listed leaf")
        };

        // The listing hands the table the leaf it found, composed as its name
        // declares; the table holds that handle and a read through it is a
        // read on it.
        let held = Holder::from(catalog.table("eu.fills").expect("the table"));
        let direct = listed().into_declared_media();
        let through_handle = filesystem.costs(|| {
            assert_eq!(held.read_arrow_reader(&options).expect("a read").count(), 1);
        });
        let on_the_leaf = filesystem.costs(|| {
            assert_eq!(
                direct.read_arrow_reader(&options).expect("a read").count(),
                1
            );
        });
        assert_eq!(
            [through_handle.as_str(), on_the_leaf.as_str()],
            [
                "file_info=1 open_input_stream=1",
                "file_info=1 open_input_stream=1"
            ],
            "through the table's handle, on the listed leaf it holds"
        );
        let field_through = filesystem.costs(|| {
            catalog
                .table("eu.fills")
                .expect("the table")
                .field()
                .expect("the schema");
        });
        let field_direct = filesystem.costs(|| {
            // What the path costs - the root listing, the `metadata/` listing
            // that tells `eu` from a table format, the listing of `eu`, and
            // the matched leaf's role asked once - then the schema.
            folder.ls(false, false).count();
            folder
                .child_by_path("eu/metadata")
                .expect("metadata")
                .ls(false, false)
                .count();
            let leaf = listed();
            assert!(!leaf.is_container());
            leaf.into_declared_media()
                .read_arrow_field(&options)
                .expect("the schema");
        });
        assert_eq!(
            [field_through.as_str(), field_direct.as_str()],
            [
                "file_info=2 list=3 open_input_stream=1",
                "file_info=2 list=3 open_input_stream=1"
            ],
            "the schema through a resolved path, on the leaf"
        );

        // A clone starts unresolved: its first verb rebuilds the handle from
        // the leaf's binding as a location, and costs what that located
        // handle costs - one role resolution the leaf in hand had paid.
        let rebuilt = Holder::from(catalog.table("eu.fills").expect("the table").clone());
        let bound = leaf().bound_location().expect("bound").clone();
        let through_clone = filesystem.costs(|| {
            assert_eq!(
                rebuilt.read_arrow_reader(&options).expect("a read").count(),
                1
            );
        });
        let on_the_location = filesystem.costs(|| {
            // Composing the located leaf as its name declares is part of what
            // the clone's first verb does, so it is measured beside the read.
            let located = yggdryl::fs::located(bound).into_declared_media();
            assert_eq!(
                located.read_arrow_reader(&options).expect("a read").count(),
                1
            );
        });
        assert_eq!(
            [through_clone.as_str(), on_the_location.as_str()],
            [
                "file_info=3 open_input_stream=1",
                "file_info=3 open_input_stream=1"
            ],
            "through a clone's rebuilt handle, on the located leaf"
        );
    }
}

#[test]
fn a_log_handler_publishes_with_one_append_and_holds_back_nothing_it_owes() {
    use yggdryl::IOMode;
    use yggdryl::logging::{FileHandler, Handler, Level, Record};

    let handle = source(b"", "file:///logs/feed.log");
    let calls = Arc::clone(handle.calls());
    let handler = FileHandler::new(handle);
    costs("building a handler", &calls, "none", || {});
    costs("the first record", &calls, "append_bytes=1 open=1", || {
        handler.handle(&Record::new("feed", Level::INFO, &"opened"));
    });
    costs("each later record", &calls, "append_bytes=1", || {
        handler.handle(&Record::new("feed", Level::INFO, &"tick"));
    });
    costs("a flush holding nothing", &calls, "none", || {
        handler.flush().expect("a flush");
    });
    costs("a close", &calls, "close=1", || {
        handler.close().expect("a close");
    });

    let handle = source(b"", "file:///logs/batched.log");
    let calls = Arc::clone(handle.calls());
    let handler = FileHandler::new(handle).with_capacity(64);
    // Each line its message alone, `tick\n`: five bytes against the capacity.
    handler.set_formatter(yggdryl::logging::Formatter::default());
    costs("records held under the capacity", &calls, "none", || {
        for _ in 0..5 {
            handler.handle(&Record::new("feed", Level::INFO, &"tick"));
        }
    });
    costs(
        "the record reaching the capacity",
        &calls,
        "append_bytes=1 open=1",
        || {
            for _ in 0..12 {
                handler.handle(&Record::new("feed", Level::INFO, &"tick"));
            }
        },
    );
    costs(
        "a record at the flush level",
        &calls,
        "append_bytes=1",
        || {
            handler.handle(&Record::new("feed", Level::ERROR, &"rejected"));
        },
    );
    costs(
        "a close holding records",
        &calls,
        "append_bytes=1 close=1",
        || {
            handler.handle(&Record::new("feed", Level::INFO, &"held"));
            handler.close().expect("a close");
        },
    );

    let handle = source(b"yesterday\n", "file:///logs/replaced.log");
    let calls = Arc::clone(handle.calls());
    let handler = FileHandler::new(handle)
        .with_mode(IOMode::Overwrite)
        .expect("an overwrite");
    costs(
        "an overwrite's first publish",
        &calls,
        "write_all_bytes=1 open=1",
        || {
            handler.handle(&Record::new("feed", Level::INFO, &"today"));
        },
    );
    costs(
        "an overwrite's later publish",
        &calls,
        "append_bytes=1",
        || {
            handler.handle(&Record::new("feed", Level::INFO, &"later"));
        },
    );
}
