//! Exchange objects with Google Cloud Storage, both directions.
//!
//! `fake-gcs-server` is the reference implementation here. What it checks is
//! the *shape* of every request: the `/storage/v1` and `/upload/storage/v1`
//! paths, the object name escaped into one path segment, `alt=media` switching
//! metadata for bytes, the `multipart/related` framing of a small write, and
//! the resumable protocol's `Content-Range` and 308 answers. What it does not
//! check is the bearer token, because it accepts any - so this suite proves the
//! dialect and not the identity, and says so rather than implying more.
//!
//! `YGGDRYL_GCS_ENDPOINT` is what turns the suite on; without it every test
//! prints `SKIPPED` and passes, and the driver fails on that word.

use yggdryl::holder::object::{GoogleOptions, ObjectOptions, Provider};
use yggdryl::{IOBase, IOFolder, IOKind};

/// The bucket both sides exchange through.
const BUCKET: &str = "yggdryl-interop";
/// The prefix this half writes under.
const FROM_RUST: &str = "from-rust";
/// The prefix the reference client writes under.
const FROM_REFERENCE: &str = "from-google";

/// The endpoint the suite runs against, or `None` to skip.
fn endpoint() -> Option<String> {
    std::env::var("YGGDRYL_GCS_ENDPOINT")
        .ok()
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
}

/// Options addressing that endpoint with a token the emulator does not read.
fn options() -> ObjectOptions {
    ObjectOptions::default()
        .with_environment(false)
        .with_endpoint(endpoint().expect("an endpoint"))
        .with_google(
            GoogleOptions::default()
                .with_access_token("ya29.emulator")
                .with_project("yggdryl-interop"),
        )
}

/// The object `key` names in the exchange bucket.
fn object(key: &str) -> yggdryl::holder::object::File {
    yggdryl::holder::object::file_at_with(Provider::Google, BUCKET, key, options())
        .expect("an object handle")
}

/// The prefix `key` names in the exchange bucket.
fn prefix(key: &str) -> yggdryl::holder::object::Folder {
    yggdryl::holder::object::folder_at_with(Provider::Google, BUCKET, key, options())
        .expect("a prefix handle")
}

/// Say why nothing ran, in the word the driver greps for.
fn skipped(what: &str) {
    println!("SKIPPED: {what}");
}

/// The bucket the exchange runs in, created if the driver did not.
fn bucket() -> yggdryl::holder::object::Folder {
    let root = yggdryl::holder::object::folder_at_with(Provider::Google, BUCKET, "", options())
        .expect("a bucket handle");
    if !root.folder_exists() {
        root.create_folder().expect("a bucket");
    }
    root
}

#[test]
fn an_object_written_here_reads_back_whole_and_by_range() {
    if endpoint().is_none() {
        skipped("YGGDRYL_GCS_ENDPOINT names no emulator");
        return;
    }
    bucket();
    let mut handle = object(&format!("{FROM_RUST}/quotes.csv"));
    handle
        .write_all_bytes(b"symbol,price\nAAPL,187.23\n")
        .expect("a write");

    assert_eq!(
        handle.read_all_bytes().expect("a read"),
        b"symbol,price\nAAPL,187.23\n"
    );
    // `size` reads the object resource, whose `size` is a JSON string because
    // it is 64-bit - the one field a reader gets wrong.
    assert_eq!(handle.size(), 25);
    assert_eq!(handle.read_range_bytes(13, 4).expect("a range"), b"AAPL");
    assert_eq!(handle.kind(), IOKind::File);
}

#[test]
fn a_key_whose_separators_are_escaped_into_one_segment_survives() {
    if endpoint().is_none() {
        skipped("YGGDRYL_GCS_ENDPOINT names no emulator");
        return;
    }
    bucket();
    // The JSON API names an object in a single path segment, so every `/` in
    // the key is escaped - the opposite of the XML API's rule, and where a
    // hand-written client usually addresses the wrong object.
    for name in [
        "a b/spaced.txt",
        "plus+sign.txt",
        "equals=sign.txt",
        "unicode-é.txt",
    ] {
        let key = format!("{FROM_RUST}/{name}");
        let mut handle = object(&key);
        handle.write_all_bytes(name.as_bytes()).expect("a write");
        assert_eq!(handle.key(), key);
        assert_eq!(handle.read_all_bytes().expect("a read"), name.as_bytes());
    }
}

#[test]
fn a_large_object_is_sent_as_a_resumable_session_of_chunks() {
    if endpoint().is_none() {
        skipped("YGGDRYL_GCS_ENDPOINT names no emulator");
        return;
    }
    bucket();
    // Every chunk but the last is a multiple of 256 KiB, and the answer to each
    // is 308 until the last, which is the protocol's own use of the code.
    let bytes: Vec<u8> = (0..9 * 1024 * 1024)
        .map(|index| (index % 251) as u8)
        .collect();
    let mut handle = yggdryl::holder::object::file_at_with(
        Provider::Google,
        BUCKET,
        &format!("{FROM_RUST}/large.bin"),
        options()
            .with_part_size(4 * 1024 * 1024)
            .with_multipart_threshold(1024 * 1024),
    )
    .expect("an object handle");
    handle.write_all_bytes(&bytes).expect("a write");
    assert_eq!(handle.size(), bytes.len() as u64);
    assert_eq!(handle.read_all_bytes().expect("a read"), bytes);
}

#[test]
fn a_listing_reads_the_json_page_the_emulator_answers() {
    if endpoint().is_none() {
        skipped("YGGDRYL_GCS_ENDPOINT names no emulator");
        return;
    }
    bucket();
    for name in [
        "year=2026/a.parquet",
        "year=2026/b.parquet",
        "year=2027/c.parquet",
    ] {
        object(&format!("{FROM_RUST}/lake/{name}"))
            .write_all_bytes(b"PAR1")
            .expect("a write");
    }
    let lake = prefix(&format!("{FROM_RUST}/lake"));
    let mut rolled: Vec<String> = lake
        .ls(false, false)
        .filter_map(Result::ok)
        .filter_map(|entry| entry.url().map(|url| url.to_string()))
        .collect();
    rolled.sort();
    assert_eq!(rolled.len(), 2, "{rolled:?}");
    assert!(rolled[0].ends_with("year=2026/"), "{rolled:?}");

    assert_eq!(lake.ls(true, false).filter_map(Result::ok).count(), 5);
}

#[test]
fn what_the_reference_client_wrote_reads_back_here() {
    if endpoint().is_none() {
        skipped("YGGDRYL_GCS_ENDPOINT names no emulator");
        return;
    }
    let handle = object(&format!("{FROM_REFERENCE}/quotes.csv"));
    if !handle.exists() {
        skipped("the reference client wrote nothing to read");
        return;
    }
    assert_eq!(
        handle.read_all_bytes().expect("a read"),
        b"symbol,price\nMSFT,411.10\n"
    );
    for name in ["a b/spaced.txt", "plus+sign.txt", "unicode-é.txt"] {
        let held = object(&format!("{FROM_REFERENCE}/{name}"));
        assert!(held.exists(), "{name}");
        assert_eq!(held.read_all_bytes().expect("a read"), name.as_bytes());
    }
}

#[test]
fn removing_a_prefix_is_one_listing_and_one_batch() {
    if endpoint().is_none() {
        skipped("YGGDRYL_GCS_ENDPOINT names no emulator");
        return;
    }
    bucket();
    for index in 0..4 {
        object(&format!("{FROM_RUST}/scratch/part-{index}.bin"))
            .write_all_bytes(b"x")
            .expect("a write");
    }
    let mut scratch = prefix(&format!("{FROM_RUST}/scratch"));
    scratch.folder_remove(true).expect("a removal");
    assert_eq!(
        prefix(&format!("{FROM_RUST}/scratch"))
            .ls(true, false)
            .filter_map(Result::ok)
            .count(),
        0
    );
}
