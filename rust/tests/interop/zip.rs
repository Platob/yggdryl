//! The ZIP exchange with an external implementation.
//!
//! `scripts/check_zip_interop.py` drives this target twice around a Python
//! `zipfile` round trip: the first run writes an archive `zipfile` must read,
//! the second reads an archive `zipfile` wrote. The reading half prints
//! `SKIPPED` when the external archive is absent - the driver fails on that
//! word - so a skipped half can never read as a pass.

use yggdryl::holder::{Holder, zip};
use yggdryl::{Codec, IOBase};

/// Where the exchange files live, shared with the Python driver.
fn exchange_dir() -> std::path::PathBuf {
    let mut path = std::env::current_dir().expect("a working directory");
    // Under `cargo test` the working directory is `rust/`.
    path.push("target");
    path.push("zip-interop");
    path
}

/// The members both sides assert, in name order.
///
/// The names cover a nested path, a name that needs percent-encoding in a
/// fragment, and a member whose payload compresses well enough that a stored
/// and a deflated copy differ on the wire.
fn expected_members() -> Vec<(&'static str, Vec<u8>)> {
    vec![
        ("empty.bin", Vec::new()),
        ("notes/read me.txt", b"symbol,price\n".to_vec()),
        (
            "trades/2024/eu.csv",
            b"symbol,price\nAAPL,187.23\n".repeat(64),
        ),
        (
            "trades/2024/us.csv",
            b"symbol,price\nMSFT,412.10\n".to_vec(),
        ),
    ]
}

#[test]
fn writes_an_archive_for_the_external_reader() {
    let dir = exchange_dir();
    std::fs::create_dir_all(&dir).expect("the exchange directory");
    let path = dir.join("from-rust.zip");
    let _ = std::fs::remove_file(&path);

    let root = zip::mount(Holder::file(&path).expect("a local archive"));
    let archive = match &root {
        Holder::ZipFolder(folder) => folder.archive(),
        other => panic!("expected an archive root, got {other:?}"),
    };
    // One stored member and the rest deflated, so the external reader has to
    // handle both methods in one archive.
    for (name, payload) in expected_members() {
        let codec = if name == "trades/2024/us.csv" {
            Codec::Identity
        } else {
            Codec::Deflate
        };
        archive
            .write_member_with(name, &payload, codec)
            .expect("the member writes");
    }
    archive
        .create_directory("empty-directory")
        .expect("the directory record writes");
    archive
        .set_comment(b"written by yggdryl")
        .expect("the archive comment");
    archive.flush().expect("the directory publishes");
    println!("zip-interop: wrote");
}

#[test]
fn reads_the_archive_the_external_writer_produced() {
    let path = exchange_dir().join("from-python.zip");
    if !path.exists() {
        println!("zip-interop: SKIPPED (no {})", path.display());
        return;
    }
    let root = zip::mount(Holder::file(&path).expect("a local archive"));
    for (name, payload) in expected_members() {
        let member = root.child_by_path(name).expect("a member");
        assert_eq!(
            member.read_all_bytes().expect("the member reads"),
            payload,
            "member {name}"
        );
        // The location a member reports opens that member again.
        let located = zip::from_url(member.url().expect("a member url"))
            .expect("the member location resolves");
        assert_eq!(
            located.read_all_bytes().expect("the member reads"),
            payload,
            "member {name} through its url"
        );
    }

    // The tree Python wrote lists as the tree its member names describe.
    let names: Vec<String> = root
        .ls(true, false)
        .map(|entry| {
            let entry = entry.expect("a listed member");
            let url = entry.url().expect("a member url");
            url.fragment(true)
                .expect("a member fragment")
                .map(std::borrow::Cow::into_owned)
                .unwrap_or_default()
        })
        .collect();
    for (name, _) in expected_members() {
        assert!(names.iter().any(|listed| listed == name), "{names:?}");
    }
    assert!(
        names.iter().any(|listed| listed == "trades/2024"),
        "{names:?}"
    );

    let archive = match &root {
        Holder::ZipFolder(folder) => folder.archive(),
        other => panic!("expected an archive root, got {other:?}"),
    };
    assert_eq!(
        archive.comment().expect("the comment"),
        b"written by python"
    );
    println!("zip-interop: read");
}

#[test]
fn appends_to_the_archive_the_external_writer_produced() {
    let path = exchange_dir().join("from-python.zip");
    if !path.exists() {
        println!("zip-interop: SKIPPED (no {})", path.display());
        return;
    }
    let target = exchange_dir().join("round-trip.zip");
    std::fs::copy(&path, &target).expect("the archive copies");

    // Updating an archive another implementation wrote is the case that
    // exercises reading its directory and writing one it must read back.
    let root = zip::mount(Holder::file(&target).expect("a local archive"));
    root.child_by_path("trades/2024/eu.csv")
        .expect("a member")
        .write_all_bytes(b"symbol,price\nAAPL,999.99\n")
        .expect("the member rewrites");
    root.child_by_path("added/by-rust.txt")
        .expect("a member")
        .write_all_bytes(b"appended")
        .expect("the member writes");
    let mut removed = root.child_by_path("empty.bin").expect("a member");
    removed.remove(false).expect("the member is removed");

    assert_eq!(
        root.child_by_path("trades/2024/us.csv")
            .expect("a member")
            .read_all_bytes()
            .expect("the untouched member reads"),
        b"symbol,price\nMSFT,412.10\n",
    );
    println!("zip-interop: updated");
}

#[test]
fn compacts_the_streamed_archive_the_external_writer_produced() {
    let path = exchange_dir().join("from-python-streamed.zip");
    if !path.exists() {
        println!("zip-interop: SKIPPED (no {})", path.display());
        return;
    }
    let target = exchange_dir().join("round-trip-streamed.zip");
    std::fs::copy(&path, &target).expect("the archive copies");

    // Every member here promises its sizes after its bytes rather than in its
    // local header. Removing one compacts the rest, which is where those
    // promises have to be settled into the headers that moved.
    let root = zip::mount(Holder::file(&target).expect("a local archive"));
    root.child_by_path("empty.bin")
        .expect("a member")
        .remove(false)
        .expect("the member is removed");
    root.child_by_path("trades/2024/eu.csv")
        .expect("a member")
        .write_all_bytes(b"symbol,price\nAAPL,999.99\n")
        .expect("the member rewrites");

    assert_eq!(
        root.child_by_path("trades/2024/us.csv")
            .expect("a member")
            .read_all_bytes()
            .expect("the untouched member reads"),
        b"symbol,price\nMSFT,412.10\n",
    );
    println!("zip-interop: compacted");
}
