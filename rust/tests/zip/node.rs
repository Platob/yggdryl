//! `rust/src/zip/node.rs`: one directory inside an archive, the archive root
//! included - the stream of the members beneath it.
//!
//! A directory holds no bytes of its own; what it streams is its members',
//! read one after another in the index's order, each through the archive.

use yggdryl::holder::{Buffer, Holder};
use yggdryl::zip::{ZipArchive, ZipNode};
use yggdryl::{ByteStream, Codec, Error, IOBase, Result};

/// The root of a fresh in-memory archive holding `members`, published.
fn archive(members: &[(&str, &[u8])]) -> ZipNode {
    let root = ZipArchive::new(Holder::buffer(Buffer::new())).mount();
    for (name, bytes) in members {
        root.archive()
            .write_member(name, bytes)
            .expect("the member writes");
    }
    root.archive().flush().expect("the directory publishes");
    root
}

/// Every chunk a stream yields, failing on the first error.
fn chunks(stream: ByteStream<'_>) -> Vec<Vec<u8>> {
    stream.collect::<Result<Vec<_>>>().expect("the stream")
}

/// Three members at two depths under `logs/`, one without a final newline,
/// one empty and one hidden, beside a member outside the directory.
const MEMBERS: &[(&str, &[u8])] = &[
    ("logs/a.log", b"a1\na2\n"),
    ("logs/b.log", b"b1\nb2"),
    ("logs/empty.log", b""),
    ("logs/.hidden", b"h\n"),
    ("logs/sub/c.log", b"c1\n"),
    ("other/x.log", b"x1\n"),
];

/// The members of [`MEMBERS`] beneath `logs/`, in index order, end to end.
const STREAMED: &[u8] = b"a1\na2\nb1\nb2c1\n";

#[test]
fn a_directory_refuses_a_stream_that_could_never_yield_a_byte() {
    let logs = archive(MEMBERS).as_node("logs").expect("a directory");
    let refused = logs.pstream_bytes(0, 0).expect_err("a refusal");
    assert!(
        matches!(&refused, Error::Io(error) if error.kind() == std::io::ErrorKind::InvalidInput),
        "{refused:?}"
    );
}

#[test]
fn a_directory_streams_every_member_beneath_it_end_to_end() {
    let logs = archive(MEMBERS).as_node("logs").expect("a directory");

    // Sorted by name and recursive; `b.log` runs straight into `sub/c.log`,
    // the empty member adds nothing, and neither the hidden member nor the
    // one outside the prefix is read.
    let streamed = chunks(logs.pstream_bytes(0, 4).expect("a stream"));
    assert_eq!(streamed.concat(), STREAMED);
    assert_eq!(
        streamed.iter().map(Vec::len).collect::<Vec<_>>(),
        [4, 4, 4, 2],
        "full chunks across the member boundaries, none empty"
    );
    assert_eq!(logs.read_all_bytes().expect("a whole read"), STREAMED);
    assert_eq!(
        logs.read_range_bytes(7, 5).expect("a ranged read"),
        &STREAMED[7..12]
    );

    // The directory itself holds no positional bytes and is no value.
    let mut window = [0_u8; 8];
    assert_eq!(logs.pread(0, &mut window).expect("a read"), 0);
    assert_eq!(logs.size(), 0);
    assert!(!logs.is_atomic());
}

#[test]
fn the_archive_root_streams_every_member_it_holds() {
    let root = archive(MEMBERS);
    assert_eq!(
        root.read_all_bytes().expect("a whole read"),
        b"a1\na2\nb1\nb2c1\nx1\n"
    );
}

#[test]
fn a_directory_stream_starts_at_a_content_position_and_decodes_what_its_members_declare() {
    let coded = Codec::Gzip.dump(b"b1\nb2\n").expect("an encoding");
    let logs = archive(&[
        ("logs/a.log", b"a1\n"),
        ("logs/b.log.gz", &coded),
        ("logs/c.log", b"c1\n"),
    ])
    .as_node("logs")
    .expect("a directory");
    let content: &[u8] = b"a1\nb1\nb2\nc1\n";

    // The member's own name declares gzip, so it contributes its text.
    assert_eq!(
        chunks(logs.pstream_bytes(0, 5).expect("a stream")).concat(),
        content
    );
    for position in [2, 4, 9, 11, 12] {
        assert_eq!(
            chunks(logs.pstream_bytes(position, 5).expect("a stream")).concat(),
            &content[position as usize..],
            "from {position}"
        );
    }
}

#[test]
fn a_failing_member_arrives_after_the_prefix_and_the_stream_is_fused() {
    let logs = archive(&[
        ("logs/a.log", b"a1\n"),
        ("logs/b.log.gz", b"these bytes are not gzip"),
        ("logs/c.log", b"c1\n"),
    ])
    .as_node("logs")
    .expect("a directory");

    let mut stream = logs.pstream_bytes(0, 16).expect("a stream");
    assert_eq!(stream.next().expect("the prefix").expect("bytes"), b"a1\n");
    assert!(stream.next().expect("the failure").is_err());
    assert!(stream.next().is_none(), "fused after the failure");
    assert!(stream.next().is_none());
}

#[test]
fn a_directory_holding_no_members_streams_nothing() {
    let empty = ZipArchive::new(Holder::buffer(Buffer::new())).mount();
    assert!(
        empty
            .pstream_bytes(0, 4)
            .expect("a stream")
            .next()
            .is_none()
    );

    // A directory record with nothing but a hidden member beneath it.
    let root = archive(&[("logs/.hidden", b"h\n")]);
    root.archive()
        .create_directory("logs/sub")
        .expect("a record");
    root.archive().flush().expect("publishes");
    let logs = root.as_node("logs").expect("a directory");
    assert!(logs.pstream_bytes(0, 4).expect("a stream").next().is_none());
    assert!(logs.read_all_bytes().expect("a whole read").is_empty());
}

#[test]
fn a_directory_stream_outlives_the_handles_that_built_it() {
    let root = archive(MEMBERS);
    let logs = root.as_node("logs").expect("a directory");
    let stream = ByteStream::from_container(&logs, 3, 4).expect("a stream");
    drop(logs);
    drop(root);
    assert_eq!(chunks(stream).concat(), &STREAMED[3..]);
}

#[test]
fn a_directory_stream_costs_what_reading_each_member_costs() {
    let root = archive(MEMBERS);
    let logs = root.as_node("logs").expect("a directory");
    let reads = |read: &dyn Fn()| {
        let before = root.archive().handle_reads();
        read();
        root.archive().handle_reads() - before
    };

    // The same members read one by one, each whole through its own handle.
    let members = reads(&|| {
        for name in [
            "logs/a.log",
            "logs/b.log",
            "logs/empty.log",
            "logs/sub/c.log",
        ] {
            root.as_leaf(name)
                .expect("a member")
                .read_all_bytes()
                .expect("a whole read");
        }
    });
    // Listing the directory reads no member byte, so the stream costs the
    // members' reads and nothing besides them - and a member the position
    // covers whole is passed by its indexed size, unread.
    let streamed = reads(&|| {
        assert_eq!(
            chunks(logs.pstream_bytes(0, 4).expect("a stream")).concat(),
            STREAMED
        );
    });
    assert_eq!(streamed, members);
    assert_eq!(streamed, 4, "one read per stored member, the empty one too");
    let skipped = reads(&|| {
        assert_eq!(
            chunks(logs.pstream_bytes(6, 4).expect("a stream")).concat(),
            &STREAMED[6..]
        );
    });
    assert_eq!(skipped, 3, "the first member passed by its size");
}
