//! `rust/src/zip/path.rs`: one member location, whatever it turns out to be -
//! a member read as its bytes, a prefix read as the stream of the members
//! beneath it.

use yggdryl::holder::{Buffer, Holder};
use yggdryl::zip::{ZipArchive, ZipNode};
use yggdryl::{ByteStream, IOBase, IOKind, Result};

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

#[test]
fn a_location_at_a_prefix_streams_the_members_beneath_it() {
    let root = archive(&[
        ("logs/a.log", b"a1\na2"),
        ("logs/sub/b.log", b"b1\n"),
        ("other.log", b"o\n"),
    ]);
    let logs = root.child_by_path("logs").expect("the location");
    assert!(matches!(logs, Holder::ZipPath(_)), "{logs:?}");
    let streamed: &[u8] = b"a1\na2b1\n";

    assert_eq!(logs.kind(), IOKind::Directory);
    let read = chunks(logs.pstream_bytes(0, 3).expect("a stream"));
    assert_eq!(read.concat(), streamed);
    assert_eq!(
        read.iter().map(Vec::len).collect::<Vec<_>>(),
        [3, 3, 2],
        "full chunks across the member boundary"
    );
    assert_eq!(
        chunks(logs.pstream_bytes(4, 3).expect("a stream")).concat(),
        &streamed[4..]
    );
    assert_eq!(logs.read_all_bytes().expect("a whole read"), streamed);
    assert_eq!(
        logs.read_range_bytes(2, 4).expect("a ranged read"),
        &streamed[2..6]
    );
    assert_eq!(logs.size(), 0, "a prefix holds no bytes of its own");
}

#[test]
fn a_location_at_a_member_streams_that_member_alone() {
    let root = archive(&[("logs/a.log", b"a1\na2"), ("logs/a.log.bak", b"old\n")]);
    let member = root.child_by_path("logs/a.log").expect("the location");

    assert_eq!(member.kind(), IOKind::File);
    assert_eq!(
        chunks(member.pstream_bytes(0, 2).expect("a stream")).concat(),
        b"a1\na2"
    );
    assert_eq!(
        chunks(member.pstream_bytes(3, 2).expect("a stream")).concat(),
        b"a2"
    );
    assert_eq!(member.read_all_bytes().expect("a whole read"), b"a1\na2");
}

#[test]
fn a_location_that_is_a_member_and_a_prefix_streams_what_is_beneath_it() {
    let root = archive(&[("a", b"member bytes"), ("a/in.txt", b"beneath\n")]);

    // What the location is decides every verb: it lists as a container, so
    // its stream is the member beneath the prefix, never the member of the
    // same name.
    let held = root.child_by_path("a").expect("the location");
    assert_eq!(held.kind(), IOKind::Directory);
    assert_eq!(held.read_all_bytes().expect("a whole read"), b"beneath\n");
    assert_eq!(
        chunks(held.pstream_bytes(0, 4).expect("a stream")).concat(),
        b"beneath\n"
    );

    // The member stays reachable through the role that names it.
    assert_eq!(
        root.as_leaf("a")
            .expect("a member")
            .read_all_bytes()
            .expect("bytes"),
        b"member bytes"
    );
}
