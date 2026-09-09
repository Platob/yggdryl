//! Exchange blobs with Azure Blob Storage, both directions.
//!
//! Azurite is the reference implementation here, and it is the one that matters
//! most for this store: it recomputes the Shared Key `StringToSign` exactly as
//! the service does and refuses anything that does not match, so a signature
//! this client builds wrong is a `403` rather than a silent agreement between
//! two halves of the same mistake. The order of the thirteen signed lines, the
//! empty `Content-Length` for a zero-length body, the account appearing twice
//! in the canonicalized resource of a path-style endpoint - each is checked
//! here by a server that did not read this crate's source.
//!
//! `YGGDRYL_AZURE_ENDPOINT` is what turns the suite on; without it every test
//! prints `SKIPPED` and passes, and the driver fails on that word.

use yggdryl::holder::object::{AzureOptions, ObjectOptions, Provider};
use yggdryl::{IOBase, IOFolder, IOKind};

/// The container both sides exchange through.
const CONTAINER: &str = "yggdryl-interop";
/// The prefix this half writes under.
const FROM_RUST: &str = "from-rust";
/// The prefix the reference client writes under.
const FROM_REFERENCE: &str = "from-azure";

/// The endpoint the suite runs against, or `None` to skip.
fn endpoint() -> Option<String> {
    std::env::var("YGGDRYL_AZURE_ENDPOINT")
        .ok()
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
}

/// The account the driver provisioned, which is Azurite's published one unless
/// the environment names another.
fn account() -> String {
    std::env::var("AZURE_STORAGE_ACCOUNT_NAME")
        .unwrap_or_else(|_| yggdryl::holder::object::azure::DEVELOPMENT_ACCOUNT.to_owned())
}

/// The shared key it is signed with.
fn key() -> String {
    std::env::var("AZURE_STORAGE_ACCOUNT_KEY")
        .unwrap_or_else(|_| yggdryl::holder::object::azure::DEVELOPMENT_KEY.to_owned())
}

/// Options addressing that endpoint with the key the driver set.
fn options() -> ObjectOptions {
    ObjectOptions::default()
        .with_environment(false)
        .with_endpoint(endpoint().expect("an endpoint"))
        .with_azure(
            AzureOptions::default()
                .with_account(account())
                .with_account_key(key()),
        )
}

/// The blob `key` names in the exchange container.
fn blob(key: &str) -> yggdryl::holder::object::File {
    yggdryl::holder::object::file_at_with(Provider::Azure, CONTAINER, key, options())
        .expect("a blob handle")
}

/// The prefix `key` names in the exchange container.
fn prefix(key: &str) -> yggdryl::holder::object::Folder {
    yggdryl::holder::object::folder_at_with(Provider::Azure, CONTAINER, key, options())
        .expect("a prefix handle")
}

/// Say why nothing ran, in the word the driver greps for.
fn skipped(what: &str) -> bool {
    println!("SKIPPED: {what}");
    true
}

/// The container the exchange runs in, created if the driver did not.
fn container() -> yggdryl::holder::object::Folder {
    let root = yggdryl::holder::object::folder_at_with(Provider::Azure, CONTAINER, "", options())
        .expect("a container handle");
    if !root.folder_exists() {
        root.create_folder().expect("a container");
    }
    root
}

#[test]
fn a_signature_azurite_recomputes_is_the_one_this_client_built() {
    if endpoint().is_none() {
        skipped("YGGDRYL_AZURE_ENDPOINT names no Azurite");
        return;
    }
    // Azurite verifies the signature itself, so reaching the container at all
    // is the assertion: a wrong `StringToSign` is a 403 and never a 200.
    let root = container();
    assert!(root.folder_exists());
}

#[test]
fn a_blob_written_here_reads_back_whole_and_by_range() {
    if endpoint().is_none() {
        skipped("YGGDRYL_AZURE_ENDPOINT names no Azurite");
        return;
    }
    container();
    let mut handle = blob(&format!("{FROM_RUST}/quotes.csv"));
    handle
        .write_all_bytes(b"symbol,price\nAAPL,187.23\n")
        .expect("a write");

    assert_eq!(
        handle.read_all_bytes().expect("a read"),
        b"symbol,price\nAAPL,187.23\n"
    );
    assert_eq!(handle.size(), 25);
    // A ranged read carries `x-ms-range`, which the signature covers through
    // the canonicalized headers rather than through the `Range` line.
    assert_eq!(handle.read_range_bytes(13, 4).expect("a range"), b"AAPL");
    assert_eq!(handle.kind(), IOKind::File);
}

#[test]
fn a_key_that_spells_differently_as_a_url_and_as_a_name_survives() {
    if endpoint().is_none() {
        skipped("YGGDRYL_AZURE_ENDPOINT names no Azurite");
        return;
    }
    container();
    // Each of these means something different as a URL component, as a signed
    // canonical path, and as a raw name.
    for name in [
        "a b/spaced.txt",
        "plus+sign.txt",
        "equals=sign.txt",
        "unicode-é.txt",
    ] {
        let key = format!("{FROM_RUST}/{name}");
        let mut handle = blob(&key);
        handle.write_all_bytes(name.as_bytes()).expect("a write");
        assert_eq!(handle.key(), key);
        assert_eq!(handle.read_all_bytes().expect("a read"), name.as_bytes());
    }
}

#[test]
fn a_large_blob_is_staged_as_blocks_and_committed_as_a_list() {
    if endpoint().is_none() {
        skipped("YGGDRYL_AZURE_ENDPOINT names no Azurite");
        return;
    }
    container();
    // Azurite enforces the rule a hand-written client usually breaks: every
    // block id of one blob has to be the same length.
    let bytes: Vec<u8> = (0..9 * 1024 * 1024)
        .map(|index| (index % 251) as u8)
        .collect();
    let mut handle = yggdryl::holder::object::file_at_with(
        Provider::Azure,
        CONTAINER,
        &format!("{FROM_RUST}/large.bin"),
        options()
            .with_part_size(4 * 1024 * 1024)
            .with_multipart_threshold(1024 * 1024),
    )
    .expect("a blob handle");
    handle.write_all_bytes(&bytes).expect("a write");
    assert_eq!(handle.size(), bytes.len() as u64);
    assert_eq!(handle.read_all_bytes().expect("a read"), bytes);
}

#[test]
fn a_listing_reads_the_enumeration_azurite_answers() {
    if endpoint().is_none() {
        skipped("YGGDRYL_AZURE_ENDPOINT names no Azurite");
        return;
    }
    container();
    for name in [
        "year=2026/a.parquet",
        "year=2026/b.parquet",
        "year=2027/c.parquet",
    ] {
        blob(&format!("{FROM_RUST}/lake/{name}"))
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

    // And a whole subtree is one flat listing.
    assert_eq!(lake.ls(true, false).filter_map(Result::ok).count(), 5);
}

#[test]
fn what_the_reference_client_wrote_reads_back_here() {
    if endpoint().is_none() {
        skipped("YGGDRYL_AZURE_ENDPOINT names no Azurite");
        return;
    }
    let handle = blob(&format!("{FROM_REFERENCE}/quotes.csv"));
    if !handle.exists() {
        skipped("the reference client wrote nothing to read");
        return;
    }
    assert_eq!(
        handle.read_all_bytes().expect("a read"),
        b"symbol,price\nMSFT,411.10\n"
    );
    // Every awkward name the reference client wrote is the same object here.
    for name in ["a b/spaced.txt", "plus+sign.txt", "unicode-é.txt"] {
        let held = blob(&format!("{FROM_REFERENCE}/{name}"));
        assert!(held.exists(), "{name}");
        assert_eq!(held.read_all_bytes().expect("a read"), name.as_bytes());
    }
}

#[test]
fn removing_a_prefix_is_one_listing_and_one_batch() {
    if endpoint().is_none() {
        skipped("YGGDRYL_AZURE_ENDPOINT names no Azurite");
        return;
    }
    container();
    for index in 0..4 {
        blob(&format!("{FROM_RUST}/scratch/part-{index}.bin"))
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
