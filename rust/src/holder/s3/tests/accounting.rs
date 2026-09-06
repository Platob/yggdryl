//! What each operation costs in round trips.
//!
//! These are the no-regression tests for the property the whole backend is
//! built around. Each one states the count in its name and asserts it exactly:
//! not "at most", because a read that quietly became two requests is the
//! regression worth catching, and not "at least", because an operation that
//! stopped talking to the store at all is equally worth catching.

use super::{BUCKET, file, folder, location, path, payload, store};
use crate::holder::s3::{File, Folder};
use crate::{IOBase, IOKind};

#[test]
fn building_a_handle_costs_nothing() {
    let store = super::store();
    let client = super::client(&store);

    let file = File::new(client.clone(), location("lake/part.parquet")).expect("a handle");
    let folder = Folder::new(client.clone(), location("lake/")).expect("a handle");
    let path =
        crate::holder::s3::Path::new(client, location("lake/part.parquet")).expect("a handle");

    // Not one request between the three of them, per the laziness contract.
    assert_eq!(store.request_count(), 0);
    // Nor does asking what a handle is called, or what it holds.
    assert_eq!(file.url().to_string(), "s3://trades/lake/part.parquet");
    assert_eq!(folder.prefix(), "lake/");
    assert_eq!(path.key(), "lake/part.parquet");
    assert_eq!(file.media_type().base(), &crate::MimeType::PARQUET);
    assert!(!file.is_container());
    assert!(folder.is_container());
    assert_eq!(store.request_count(), 0);
}

#[test]
fn a_whole_read_is_one_request_and_so_is_a_ranged_one() {
    let store = store();
    let bytes = payload(4096);
    store.put(BUCKET, "lake/part.parquet", &bytes);
    let handle = file(&store, "lake/part.parquet");

    store.clear_requests();
    assert_eq!(handle.read_all_bytes().expect("the object"), bytes);
    assert_eq!(store.request_count(), 1, "a whole read is one GET");

    // A ranged read must not ask for the size first: the range answers it.
    store.clear_requests();
    let footer = handle.read_range_bytes(4088, 8).expect("the footer");
    assert_eq!(footer, &bytes[4088..]);
    assert_eq!(store.request_count(), 1, "a ranged read is one GET");
    // And it transferred the range, not the object.
    let recorded = store.requests();
    assert_eq!(recorded[0].method, "GET");
    let range = recorded[0]
        .headers
        .iter()
        .find(|(name, _)| name == "range")
        .map(|(_, value)| value.clone());
    assert_eq!(range.as_deref(), Some("bytes=4088-4095"));

    // A positional read is the same one request.
    store.clear_requests();
    let mut window = [0_u8; 16];
    assert_eq!(handle.pread(100, &mut window).expect("a window"), 16);
    assert_eq!(window, bytes[100..116]);
    assert_eq!(store.request_count(), 1);
}

#[test]
fn a_full_stream_drain_is_one_request_not_one_per_chunk() {
    let store = store();
    let bytes = payload(64 * 1024);
    store.put(BUCKET, "lake/part.parquet", &bytes);
    let handle = file(&store, "lake/part.parquet");

    store.clear_requests();
    let chunks: Vec<Vec<u8>> = handle
        .pstream_bytes(0, 4096)
        .expect("a stream")
        .collect::<crate::Result<Vec<_>>>()
        .expect("every chunk");
    assert_eq!(chunks.len(), 16, "the payload arrives in bounded pieces");
    assert_eq!(chunks.concat(), bytes);
    assert_eq!(
        store.request_count(),
        1,
        "sixteen chunks came out of one GET"
    );

    // A digest streams the same way, so it is one request over any size.
    store.clear_requests();
    let digest = handle
        .read_digest(crate::DigestAlgorithm::Xxh3)
        .expect("a digest");
    assert_eq!(digest, crate::DigestAlgorithm::Xxh3.digest(&bytes));
    assert_eq!(store.request_count(), 1, "a digest is one GET");
}

#[test]
fn a_whole_write_is_one_request_and_an_append_is_two() {
    let store = store();
    let mut handle = file(&store, "lake/part.parquet");

    store.clear_requests();
    handle.write_all_bytes(b"AAPL,187.23\n").expect("a write");
    assert_eq!(
        store.request_count(),
        1,
        "a whole write loads nothing first"
    );
    assert_eq!(store.requests()[0].method, "PUT");

    // Appending through the handle that just wrote costs only the upload:
    // it already holds what it published, so there is nothing to fetch.
    store.clear_requests();
    let offset = handle.append_bytes(b"MSFT,410.10\n").expect("an append");
    assert_eq!(offset, 12);
    assert_eq!(store.request_count(), 1, "a warm append is one PUT");
    assert_eq!(
        store.get(BUCKET, "lake/part.parquet").expect("the object"),
        b"AAPL,187.23\nMSFT,410.10\n"
    );

    // A fresh handle has to read what is there first, because S3 replaces
    // whole objects and has no append of its own.
    let mut cold = file(&store, "lake/part.parquet");
    store.clear_requests();
    cold.append_bytes(b"GOOG,180.00\n").expect("an append");
    assert_eq!(
        store.request_count(),
        2,
        "a cold append is one GET and one PUT"
    );
    assert_eq!(
        store.get(BUCKET, "lake/part.parquet").expect("the object"),
        b"AAPL,187.23\nMSFT,410.10\nGOOG,180.00\n"
    );

    // Positional writes through a fresh handle load once and publish once,
    // however many of them there are.
    let mut staged = file(&store, "lake/part.parquet");
    store.clear_requests();
    staged.pwrite(0, b"GOOG").expect("a staged write");
    staged.pwrite(4, b"!").expect("a staged write");
    assert_eq!(store.request_count(), 1, "one load, and nothing published");
    staged.flush().expect("a publish");
    assert_eq!(store.request_count(), 2, "the flush is the second request");
}

#[test]
fn a_removal_is_one_request_and_absence_is_a_success() {
    let store = store();
    store.put(BUCKET, "lake/part.parquet", b"PAR1");
    let mut handle = file(&store, "lake/part.parquet");

    store.clear_requests();
    handle.remove(false).expect("a removal");
    assert_eq!(store.request_count(), 1, "one DELETE, with no probe first");
    assert_eq!(store.requests()[0].method, "DELETE");

    // Removing again succeeds without asking whether there is anything there.
    store.clear_requests();
    handle.remove(false).expect("a second removal");
    assert_eq!(store.request_count(), 1);
    assert!(store.get(BUCKET, "lake/part.parquet").is_none());
}

#[test]
fn an_opened_object_stops_asking_for_its_metadata() {
    let store = store();
    store.put(BUCKET, "lake/part.parquet", &payload(1024));
    let mut handle = file(&store, "lake/part.parquet");

    // Closed, every size question is a fresh HEAD: that is the contract.
    store.clear_requests();
    assert_eq!(handle.size(), 1024);
    assert_eq!(handle.size(), 1024);
    assert_eq!(store.request_count(), 2, "a closed handle keeps nothing");

    store.clear_requests();
    handle.open().expect("an open");
    assert_eq!(
        store.request_count(),
        1,
        "opening is one HEAD, not a download"
    );
    assert_eq!(store.requests()[0].method, "HEAD");

    store.clear_requests();
    for _ in 0..5 {
        assert_eq!(handle.size(), 1024);
        assert_eq!(handle.kind(), IOKind::File);
        assert!(!handle.is_empty());
    }
    assert_eq!(store.request_count(), 0, "the open scope answers them all");

    // Closing drops it, so the next question is fresh again.
    handle.close().expect("a close");
    store.clear_requests();
    assert_eq!(handle.size(), 1024);
    assert_eq!(store.request_count(), 1);
}

#[test]
fn a_read_teaches_an_open_handle_the_size_it_did_not_ask_for() {
    let store = store();
    store.put(BUCKET, "lake/part.parquet", &payload(2048));
    let mut handle = file(&store, "lake/part.parquet");
    handle.open().expect("an open");

    store.clear_requests();
    let window = handle.read_range_bytes(0, 16).expect("a window");
    assert_eq!(window.len(), 16);
    // The ranged answer stated the total, so the size is already known.
    assert_eq!(handle.size(), 2048);
    assert_eq!(store.request_count(), 1, "the read was the only request");
}

#[test]
fn a_listed_object_already_knows_its_size() {
    let store = store();
    for part in 0..3 {
        store.put(BUCKET, &format!("lake/part-{part}.parquet"), &payload(512));
    }
    let lake = folder(&store, "lake/");

    store.clear_requests();
    let entries: Vec<_> = lake
        .ls(false, false)
        .collect::<crate::Result<Vec<_>>>()
        .expect("a listing");
    assert_eq!(entries.len(), 3);
    let listed = store.request_count();
    assert_eq!(listed, 1, "one page covers the level");

    // Weighing what the listing found costs nothing more.
    let total: u64 = entries.iter().map(IOBase::size).sum();
    assert_eq!(total, 3 * 512);
    assert_eq!(
        store.request_count(),
        listed,
        "a listing states every size, so nothing re-asks"
    );
}

#[test]
fn a_recursive_listing_is_one_page_not_one_request_per_directory() {
    let store = store();
    // Two years, two months each, four parts: sixteen leaves under seven
    // containers, which a per-directory walk would pay seven listings for.
    for year in ["2024", "2025"] {
        for month in ["01", "02"] {
            for part in 0..4 {
                store.put(
                    BUCKET,
                    &format!("lake/year={year}/month={month}/part-{part}.parquet"),
                    b"PAR1",
                );
            }
        }
    }
    let lake = folder(&store, "lake/");

    store.clear_requests();
    let entries: Vec<_> = lake
        .ls(true, false)
        .collect::<crate::Result<Vec<_>>>()
        .expect("a listing");
    assert_eq!(
        store.request_count(),
        1,
        "the whole subtree came out of one flat listing"
    );

    // Every leaf, and every container above them, in depth-first pre-order.
    let names: Vec<String> = entries
        .iter()
        .filter_map(|entry| entry.url().map(ToString::to_string))
        .map(|url| url.trim_start_matches("s3://trades/lake/").to_owned())
        .collect();
    assert_eq!(names.len(), 16 + 6, "sixteen leaves and six containers");
    assert_eq!(names[0], "year=2024/");
    assert_eq!(names[1], "year=2024/month=01/");
    assert_eq!(names[2], "year=2024/month=01/part-0.parquet");
    // A container is always yielded before anything beneath it.
    for (index, name) in names.iter().enumerate() {
        if let Some(parent) = name.trim_end_matches('/').rsplit_once('/') {
            let parent = format!("{}/", parent.0);
            let position = names.iter().position(|held| *held == parent);
            assert!(
                position.is_some_and(|position| position < index),
                "{parent} must precede {name}"
            );
        }
    }
}

#[test]
fn emptying_a_prefix_deletes_in_batches_rather_than_one_by_one() {
    let store = store();
    for part in 0..25 {
        store.put(BUCKET, &format!("lake/part-{part:03}.parquet"), b"PAR1");
    }
    let mut lake = folder(&store, "lake/");

    store.clear_requests();
    lake.clear().expect("an emptied prefix");
    assert_eq!(
        store.request_count(),
        2,
        "one listing and one bulk delete, not twenty-five deletes"
    );
    let recorded = store.requests();
    assert_eq!(recorded[0].method, "GET");
    assert_eq!(recorded[1].method, "POST");
    assert!(store.keys(BUCKET).is_empty());
}

#[test]
fn resolving_a_location_costs_one_listing_and_a_slash_costs_none() {
    let store = store();
    store.put(BUCKET, "lake/part.parquet", b"PAR1");

    // A trailing slash says what it is, so nothing is asked.
    let spelled = path(&store, "lake/");
    store.clear_requests();
    assert_eq!(spelled.kind(), IOKind::Directory);
    assert!(spelled.is_container());
    assert_eq!(store.request_count(), 0, "the spelling settled it");

    // A plain name is one listing, whichever of the two it turns out to be.
    let leaf = path(&store, "lake/part.parquet");
    store.clear_requests();
    assert_eq!(leaf.kind(), IOKind::File);
    assert_eq!(store.request_count(), 1);
    assert_eq!(store.requests()[0].method, "GET");

    let prefix = path(&store, "lake");
    store.clear_requests();
    assert_eq!(prefix.kind(), IOKind::Directory);
    assert_eq!(
        store.request_count(),
        1,
        "one listing settles both questions"
    );

    // And a location with nothing at it is undecided, still for one request.
    let absent = path(&store, "lake/nothing.parquet");
    store.clear_requests();
    assert_eq!(absent.kind(), IOKind::Unknown);
    assert_eq!(store.request_count(), 1);
}

#[test]
fn resolving_a_child_asks_the_store_nothing() {
    let store = store();
    let lake = folder(&store, "lake/");

    store.clear_requests();
    let child = lake
        .child_by_path("year=2026/part.parquet")
        .expect("a child");
    let container = lake.child_by_path("year=2026/").expect("a child");
    let parent = lake.parent().expect("a parent");
    assert_eq!(store.request_count(), 0, "the hierarchy is free");

    assert_eq!(
        child.url().expect("a location").to_string(),
        "s3://trades/lake/year=2026/part.parquet"
    );
    // The slash decides the role without a request, here too.
    assert!(container.is_container());
    assert_eq!(store.request_count(), 0);
    assert_eq!(
        parent.url().expect("a location").to_string(),
        "s3://trades/"
    );
}

#[test]
fn a_large_write_uploads_in_parts_and_a_small_one_does_not() {
    let store = store();
    // A threshold far below the default, so the fixture stays small while the
    // shape of the upload is the real one. The part size is clamped up to
    // S3's own 5 MiB floor, so this payload is one part.
    let bounded = || {
        super::options(&store)
            .with_multipart_threshold(512 * 1024)
            .with_part_size(5 * 1024 * 1024)
    };

    let mut small = super::file_with("lake/small.bin", bounded());
    store.clear_requests();
    small
        .write_all_bytes(&payload(1024))
        .expect("a small write");
    assert_eq!(
        store.request_count(),
        1,
        "below the threshold it is one PUT"
    );

    let bytes = payload(600 * 1024);
    let mut big = super::file_with("lake/big.bin", bounded());
    store.clear_requests();
    big.write_all_bytes(&bytes).expect("a large write");
    assert_eq!(
        store.request_count(),
        3,
        "a multipart upload is its parts plus two"
    );
    assert_eq!(
        store.get(BUCKET, "lake/big.bin").expect("the object"),
        bytes
    );
    assert_eq!(
        store.open_uploads(),
        0,
        "the upload was completed, not left open"
    );
}
