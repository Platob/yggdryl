//! What each operation costs in round trips.
//!
//! These are the no-regression tests for the property the whole backend is
//! built around. Each one states the count in its name and asserts it exactly:
//! not "at most", because a read that quietly became two requests is the
//! regression worth catching, and not "at least", because an operation that
//! stopped talking to the store at all is equally worth catching.

use super::{BUCKET, file, folder, location, path, payload, store};
use crate::object::{File, Folder};
use crate::{IOBase, IOKind};

#[test]
fn building_a_handle_costs_nothing() {
    let store = super::store();
    let client = super::client(&store);

    let file = File::new(client.clone(), location("lake/part.parquet")).expect("a handle");
    let folder = Folder::new(client.clone(), location("lake/")).expect("a handle");
    let path = crate::object::Path::new(client, location("lake/part.parquet")).expect("a handle");

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

    // What a closed handle publishes belongs to the store, and the handle
    // lets go of it: appending after that re-reads, because a value written
    // through this handle a moment ago is not evidence about what is there
    // now, and holding the payload would keep the whole object in memory for
    // as long as the handle lived.
    store.clear_requests();
    let offset = handle.append_bytes(b"MSFT,410.10\n").expect("an append");
    assert_eq!(offset, 12);
    assert_eq!(
        store.request_count(),
        2,
        "a closed handle re-reads before it appends"
    );
    assert_eq!(
        store.get(BUCKET, "lake/part.parquet").expect("the object"),
        b"AAPL,187.23\nMSFT,410.10\n"
    );

    // An *open* handle is a scope that asked for a coherent view, so it keeps
    // what it wrote and the append is the upload alone.
    let mut open = file(&store, "lake/warm.parquet");
    open.open().expect("an open");
    store.clear_requests();
    open.write_all_bytes(b"AAPL,187.23\n").expect("a write");
    open.append_bytes(b"MSFT,410.10\n").expect("an append");
    assert_eq!(
        store.request_count(),
        2,
        "one PUT each, and nothing read back"
    );
    assert!(
        store
            .requests()
            .iter()
            .all(|request| request.method == "PUT"),
        "an open handle appends to what it holds"
    );
    open.close().expect("a close");
    assert_eq!(
        store.get(BUCKET, "lake/warm.parquet").expect("the object"),
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

/// A source that answers in pieces smaller than it is asked for, and
/// remembers the most it was ever asked for at once - which is the buffer
/// the upload holds, and so the memory it costs.
struct Metered<'bytes> {
    bytes: &'bytes [u8],
    position: usize,
    largest_ask: usize,
    reads: usize,
}

impl<'bytes> Metered<'bytes> {
    fn over(bytes: &'bytes [u8]) -> Self {
        Self {
            bytes,
            position: 0,
            largest_ask: 0,
            reads: 0,
        }
    }
}

impl std::io::Read for Metered<'_> {
    fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
        self.largest_ask = self.largest_ask.max(buffer.len());
        self.reads += 1;
        let length = buffer
            .len()
            .min(self.bytes.len() - self.position)
            .min(100 * 1024);
        buffer[..length].copy_from_slice(&self.bytes[self.position..self.position + length]);
        self.position += length;
        Ok(length)
    }
}

/// A streamed upload holds one part of its source at a time: above the
/// threshold each part is read and sent before the next is read, and the
/// source is never asked for more than a part; below it the source is read
/// once. A source that ends short is refused with nothing stored.
#[test]
fn a_streamed_upload_reads_its_source_one_part_at_a_time() {
    let store = store();
    let part = 5 * 1024 * 1024;
    let bounded = || {
        super::options(&store)
            .with_multipart_threshold(512 * 1024)
            .with_part_size(part as u64)
    };

    // Six mebibytes over five-mebibyte parts: two parts, plus the create
    // and the complete. The largest read is one part, never the object.
    let bytes = payload(6 * 1024 * 1024);
    let mut big = super::file_with("lake/streamed.bin", bounded());
    let mut source = Metered::over(&bytes);
    store.clear_requests();
    big.upload_from(&mut source, bytes.len() as u64)
        .expect("a streamed upload");
    assert_eq!(
        store.request_count(),
        4,
        "a multipart upload is its parts plus two"
    );
    assert_eq!(
        store.get(BUCKET, "lake/streamed.bin").expect("the object"),
        bytes
    );
    assert_eq!(
        source.largest_ask, part,
        "the source is asked for one part at a time, never the whole"
    );
    assert!(
        source.reads > 60,
        "the fill loop takes what the source gives"
    );
    assert_eq!(store.open_uploads(), 0);

    // Below the threshold: one PUT of the source read into one buffer.
    let small = payload(1024);
    let mut file = super::file_with("lake/streamed-small.bin", bounded());
    let mut source = Metered::over(&small);
    store.clear_requests();
    file.upload_from(&mut source, small.len() as u64)
        .expect("a small streamed upload");
    assert_eq!(
        store.request_count(),
        1,
        "below the threshold it is one PUT"
    );
    assert_eq!(store.get(BUCKET, "lake/streamed-small.bin"), Some(small));

    // A source that ends before the length it declared is refused: the
    // upload is aborted, nothing is stored under the key, and dropping the
    // handle retries nothing.
    let short = payload(300 * 1024);
    let mut file = super::file_with("lake/short.bin", bounded());
    let mut source = Metered::over(&short);
    store.clear_requests();
    let error = file
        .upload_from(&mut source, 600 * 1024)
        .expect_err("a short source is refused");
    assert!(
        error.to_string().contains("expected 614400 bytes"),
        "{error}"
    );
    assert_eq!(store.get(BUCKET, "lake/short.bin"), None);
    assert_eq!(store.open_uploads(), 0, "the abandoned upload was aborted");
    let requests = store.request_count();
    drop(file);
    assert_eq!(
        store.request_count(),
        requests,
        "nothing is retried on drop"
    );
    assert_eq!(store.get(BUCKET, "lake/short.bin"), None);
}

#[test]
fn resolving_a_location_costs_one_listing_or_two_when_a_sibling_hides_the_prefix() {
    let store = store();
    store.put(BUCKET, "lake/part.parquet", b"PAR1");
    store.put(BUCKET, "lake/part/000.parquet", b"PAR1");
    store.put(BUCKET, "lake/partial", b"x");

    // An object at the location is the first key with that prefix, because a
    // key is the smallest string starting with itself.
    store.clear_requests();
    assert_eq!(path(&store, "lake/part.parquet").kind(), IOKind::File);
    assert_eq!(store.request_count(), 1, "an object settles in one listing");

    // `.` is 0x2E and `/` is 0x2F, so `lake/part.parquet` sorts between
    // `lake/part` and everything under `lake/part/`: the first answer names a
    // sibling, and only asking for the prefix by name settles it.
    store.clear_requests();
    assert_eq!(path(&store, "lake/part").kind(), IOKind::Directory);
    assert_eq!(
        store.request_count(),
        2,
        "a sibling that sorts in between costs the second listing"
    );

    // A location nothing is at or under costs the same two.
    store.clear_requests();
    assert_eq!(path(&store, "lake/part").kind(), IOKind::Directory);
    assert_eq!(store.request_count(), 2);
    store.clear_requests();
    assert_eq!(path(&store, "lake/parti").kind(), IOKind::Unknown);
    assert_eq!(store.request_count(), 2);

    // Nothing shares the name at all, so nothing is under it either.
    store.clear_requests();
    assert_eq!(path(&store, "ledger").kind(), IOKind::Unknown);
    assert_eq!(store.request_count(), 1, "an unshared name settles in one");
}

#[test]
fn naming_a_child_and_asking_whether_a_location_is_open_cost_nothing() {
    let store = store();
    store.put(BUCKET, "lake/year=2026/part.parquet", b"PAR1");
    let handle = path(&store, "lake");

    store.clear_requests();
    let child = handle
        .child_by_path("year=2026/part.parquet")
        .expect("a child");
    assert!(!handle.opened(), "nothing has opened this location");
    assert_eq!(
        store.request_count(),
        0,
        "naming a child settles nothing about either end of it"
    );
    assert_eq!(
        child.url().expect("a location").to_string(),
        "s3://trades/lake/year=2026/part.parquet"
    );
}

#[test]
fn opening_an_object_twice_asks_once() {
    let store = store();
    store.put(BUCKET, "lake/part.parquet", &payload(4096));
    let mut handle = file(&store, "lake/part.parquet");

    store.clear_requests();
    handle.open().expect("an open");
    handle.open().expect("an open");
    handle.open().expect("an open");
    assert_eq!(
        store.request_count(),
        1,
        "opening what is already open asks nothing"
    );
    assert_eq!(handle.size(), 4096);
    assert_eq!(store.request_count(), 1, "and the size is already known");
}

#[test]
fn a_ranged_digest_asks_for_the_range_and_not_the_tail() {
    let store = store();
    store.put(BUCKET, "lake/part.parquet", &payload(256 * 1024));
    let handle = file(&store, "lake/part.parquet");

    store.clear_requests();
    let digest = handle
        .read_range_digest(0, 16, crate::DigestAlgorithm::Xxh3)
        .expect("a digest");
    assert_eq!(digest, crate::DigestAlgorithm::Xxh3.digest(&payload(16)));
    assert_eq!(store.request_count(), 1, "one GET");
    let range = store.requests()[0]
        .headers
        .iter()
        .find(|(name, _)| name == "range")
        .map(|(_, value)| value.clone());
    assert_eq!(
        range.as_deref(),
        Some("bytes=0-15"),
        "the window asked for is the window wanted"
    );
}

#[test]
fn a_scan_that_reads_a_header_out_of_each_object_keeps_one_connection() {
    let store = store();
    for part in 0..6 {
        store.put(
            BUCKET,
            &format!("lake/{part:03}.parquet"),
            &payload(64 * 1024),
        );
    }
    let lake = folder(&store, "lake/");
    for leaf in lake.ls(false, false) {
        let leaf = leaf.expect("an entry");
        let mut stream = leaf.pstream_bytes(0, 4096).expect("a stream");
        let first = stream.next().expect("a chunk").expect("bytes");
        assert_eq!(first.len(), 4096);
        // The rest of the body is abandoned, which is what a header read does.
        drop(stream);
    }
    assert_eq!(
        store.connection_count(),
        1,
        "an abandoned body gives its connection back rather than burning it"
    );
}

/// What an Iceberg table costs over the store, per operation.
///
/// The table never lists `data/` and never asks a file its role or its
/// size: every file it touches is one the metadata names, so the counts
/// below are the metadata chain - the hint, the document, the manifest list,
/// the manifests - plus the data files a scan opens or a commit uploads, and
/// the one listing a commit makes to claim its version, and nothing else.
#[cfg(feature = "iceberg")]
mod iceberg {

    use crate::StructureType;
    use std::sync::Arc;

    use arrow_array::{Int64Array, RecordBatch, StringArray};

    use super::super::BUCKET;
    use super::super::server::FakeS3;
    use crate::iceberg::{
        FormatVersion, IcebergOptions, PartitionSpec, Table, WriteStaging, assign_field_ids,
        read_manifest, write_manifest,
    };
    use crate::object::Folder;
    use crate::{DataType, Field, IOBase};

    /// The requests the store handled since the last clear, by shape.
    ///
    /// A listing is a `GET` carrying `list-type=2`, counted apart from the
    /// `GET`s that read bytes, because a listing is the one request the
    /// table has no business making of a directory a manifest names.
    #[derive(Debug, Default, PartialEq, Eq)]
    struct Tally {
        total: usize,
        put: usize,
        get: usize,
        head: usize,
        list: usize,
        delete: usize,
        post: usize,
    }

    fn tally(store: &FakeS3) -> Tally {
        let mut tally = Tally::default();
        for request in store.requests() {
            tally.total += 1;
            let listing = request
                .query
                .iter()
                .any(|(name, value)| name == "list-type" && value == "2");
            match request.method.as_str() {
                "PUT" => tally.put += 1,
                "GET" if listing => tally.list += 1,
                "GET" => tally.get += 1,
                "HEAD" => tally.head += 1,
                "DELETE" => tally.delete += 1,
                "POST" => tally.post += 1,
                _ => {}
            }
        }
        tally
    }

    /// Run `operation` and answer what it cost.
    fn cost<T>(store: &FakeS3, operation: impl FnOnce() -> T) -> (T, Tally) {
        store.clear_requests();
        let answer = operation();
        (answer, tally(store))
    }

    fn schema() -> Field {
        let mut schema = StructureType::from_fields([
            DataType::Int64.required_field("id"),
            DataType::utf8().nullable_field("symbol"),
            DataType::utf8().nullable_field("venue"),
        ])
        .map(DataType::from)
        .expect("distinct columns")
        .required_field("row");
        assign_field_ids(&mut schema, 1).expect("the schema numbers");
        schema
    }

    fn rows(ids: &[i64], venues: &[&str]) -> crate::arrow::BatchReader {
        let batch = RecordBatch::try_new(
            schema().into_arrow_schema().expect("an Arrow schema"),
            vec![
                Arc::new(Int64Array::from(ids.to_vec())),
                Arc::new(StringArray::from(vec!["AAPL"; ids.len()])),
                Arc::new(StringArray::from(venues.to_vec())),
            ],
        )
        .expect("the batch matches the schema");
        crate::arrow::batch_reader(batch.schema(), [batch])
    }

    fn drain(reader: crate::arrow::BatchReader) -> usize {
        reader.map(|batch| batch.expect("a batch").num_rows()).sum()
    }

    /// Assert one operation's exact shape, printing it beside the check so
    /// a run reports the numbers the docs quote.
    fn pin(label: &str, tally: &Tally, expected: Tally) {
        println!("iceberg over s3: {label} = {tally:?}");
        assert_eq!(*tally, expected, "{label}");
    }

    /// The exact request shape a fresh table's create, commits, open and
    /// scans cost. Before staging and the leaf handles, the same sequence
    /// cost 9, 25, 40, 40, 5, 21, 9 and 29 requests: a listing to settle
    /// every handle's role, a `HEAD` for every size, and the data file's
    /// footer read back from the store after each upload.
    #[test]
    fn what_a_table_costs_over_the_store() {
        let store = super::super::store();
        let root: Folder = super::super::folder(&store, "lake/trades/");
        let schema = schema();
        let spec = PartitionSpec::identity(1, &schema, &["venue"]).expect("venue is a column");

        // The claim, the document and the hint, the one listing that
        // detects a competing claim, and the claim's removal.
        let (mut table, create) = cost(&store, || {
            Table::create(root.clone(), FormatVersion::V2, schema.clone(), spec).expect("creates")
        });
        pin(
            "create",
            &create,
            Tally {
                total: 5,
                put: 3,
                list: 1,
                delete: 1,
                ..Tally::default()
            },
        );
        assert!(
            table.write_staging().expect("resolves").folder().is_some(),
            "a remote root stages by default"
        );

        // One upload per file - the data file, the manifest, the manifest
        // list - then the hint read that re-checks the version and the
        // create's own five. Nothing reads a footer or a size back.
        let ((), append_one) = cost(&store, || {
            table.commit_append(rows(&[1], &["XNAS"])).expect("appends")
        });
        pin(
            "append one partition",
            &append_one,
            Tally {
                total: 9,
                put: 6,
                get: 1,
                list: 1,
                delete: 1,
                ..Tally::default()
            },
        );

        // Three data files, and the manifest list of the snapshot before
        // is read once to carry its manifests forward.
        let ((), append_three) = cost(&store, || {
            table
                .commit_append(rows(&[2, 3, 4], &["XNAS", "XNYS", "XLON"]))
                .expect("appends")
        });
        pin(
            "append three partitions",
            &append_three,
            Tally {
                total: 12,
                put: 8,
                get: 2,
                list: 1,
                delete: 1,
                ..Tally::default()
            },
        );

        // The plan reads the list and both manifests, the join reads the one
        // file the key bounds keep, and the commit writes one data file, its
        // manifest, the carried manifest, the list and the document.
        let merge_by = crate::Selector::from_columns(["id"]);
        let ((), upsert) = cost(&store, || {
            table
                .commit_merge(rows(&[2, 5], &["XNAS", "XNAS"]), &merge_by, true)
                .expect("merges")
        });
        pin(
            "upsert one partition of three",
            &upsert,
            Tally {
                total: 14,
                put: 7,
                get: 5,
                list: 1,
                delete: 1,
                ..Tally::default()
            },
        );

        // The hint and the document, and no listing of the directory.
        let (opened, open) = cost(&store, || Table::open(root.clone()).expect("opens"));
        pin(
            "open",
            &open,
            Tally {
                total: 2,
                get: 2,
                ..Tally::default()
            },
        );

        // The list, two manifests, four data files: one `GET` each.
        let (read, full) = cost(&store, || drain(opened.scan(None).expect("a scan")));
        assert_eq!(read, 5);
        pin(
            "full scan",
            &full,
            Tally {
                total: 7,
                get: 7,
                ..Tally::default()
            },
        );

        // The list, the one manifest the summary keeps, the one file.
        let (read, pruned) = cost(&store, || {
            drain(
                opened
                    .scan_where(&[("venue", "XNYS")], None)
                    .expect("a pruned scan"),
            )
        });
        assert_eq!(read, 1);
        pin(
            "pruned scan",
            &pruned,
            Tally {
                total: 3,
                get: 3,
                ..Tally::default()
            },
        );

        // A projection opens each file once too: the table never renamed a
        // column, so no footer is read for the names first.
        let target: Field = StructureType::from_fields([DataType::Int64.required_field("id")])
            .map(DataType::from)
            .expect("one column")
            .required_field("row");
        let (read, projected) = cost(&store, || {
            drain(opened.scan(Some(&target)).expect("a projected scan"))
        });
        assert_eq!(read, 5);
        pin(
            "projected scan",
            &projected,
            Tally {
                total: 7,
                get: 7,
                ..Tally::default()
            },
        );
        assert!(root.stats().lists >= 1);
    }

    /// A refused upload fails the commit before anything else goes out: no
    /// manifest, no manifest list, no document, no orphan under `data/`,
    /// and nothing left in the staging folder.
    #[test]
    fn a_failed_upload_publishes_nothing_and_leaves_no_staged_file() {
        let store = super::super::store();
        let root: Folder = super::super::folder(&store, "lake/trades/");
        let schema = schema();
        let spec = PartitionSpec::identity(1, &schema, &["venue"]).expect("venue is a column");
        let mut table =
            Table::create(root.clone(), FormatVersion::V2, schema, spec).expect("creates");
        let version = table.metadata_version();
        let stage = crate::local::Folder::temporary()
            .expect("the temporary folder")
            .path()
            .expect("a platform path")
            .join(format!("yggdryl-s3-staging-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&stage);
        table.set_options(
            IcebergOptions::new()
                .try_with_write_staging(
                    WriteStaging::from_str(&stage.to_string_lossy()).expect("a local folder"),
                )
                .expect("a local folder is accepted")
                .try_with_write_parallelism(1)
                .expect("one writer thread"),
        );
        let before = store.keys(BUCKET);

        // The first data file's upload is refused: nothing was published,
        // so nothing is removed - a refused `PUT` stores nothing, and no
        // `DELETE` goes out for the key it never wrote.
        store.clear_requests();
        store.fail_next(403, "AccessDenied", 1);
        let error = table
            .commit_append(rows(&[1, 2, 3], &["XLON", "XNAS", "XNYS"]))
            .expect_err("a refused upload fails the commit");
        assert!(error.to_string().contains("AccessDenied"), "{error}");
        assert_eq!(table.metadata_version(), version);
        assert!(table.current_snapshot().is_none());
        assert_eq!(
            store.keys(BUCKET),
            before,
            "no data file, manifest, list or document survives the failure"
        );
        let requests = tally(&store);
        assert_eq!(
            (requests.put, requests.post, requests.delete),
            (1, 0, 0),
            "the refused upload was the only request"
        );
        assert!(
            !stage.exists()
                || std::fs::read_dir(&stage)
                    .expect("the staging folder lists")
                    .next()
                    .is_none(),
            "the staging folder is empty"
        );

        // The second data file's upload is refused: the first was uploaded
        // and is removed again - one `DELETE`, for the one key that was
        // written - and the refused one is not.
        store.clear_requests();
        store.fail_after(1, 403, "AccessDenied", 1);
        let error = table
            .commit_append(rows(&[1, 2, 3], &["XLON", "XNAS", "XNYS"]))
            .expect_err("a refused upload fails the commit");
        assert!(error.to_string().contains("AccessDenied"), "{error}");
        assert_eq!(table.metadata_version(), version);
        assert!(table.current_snapshot().is_none());
        assert_eq!(
            store.keys(BUCKET),
            before,
            "the data file that was published is removed again"
        );
        let requests = tally(&store);
        assert_eq!(
            (requests.put, requests.post, requests.delete),
            (2, 0, 1),
            "two uploads, the second refused, and the first removed"
        );
        let removed: Vec<String> = store
            .requests()
            .into_iter()
            .filter(|request| request.method == "DELETE")
            .filter_map(|request| request.key)
            .collect();
        assert_eq!(removed.len(), 1, "{removed:?}");
        assert!(
            removed[0].starts_with("lake/trades/data/venue=XLON/"),
            "the removal names the one file that was written: {removed:?}"
        );
        assert_eq!(store.open_uploads(), 0);
        assert!(
            !stage.exists()
                || std::fs::read_dir(&stage)
                    .expect("the staging folder lists")
                    .next()
                    .is_none(),
            "the staging folder is empty after the second failure too"
        );

        // A commit after the failure is whole again, and stages nothing
        // behind it either.
        table
            .commit_append(rows(&[1, 2, 3], &["XLON", "XNAS", "XNYS"]))
            .expect("the next commit succeeds");
        assert_eq!(table.metadata_version(), version + 1);
        assert_eq!(
            store
                .keys(BUCKET)
                .iter()
                .filter(|key| key.starts_with("lake/trades/data/"))
                .count(),
            3
        );
        assert!(
            !stage.exists()
                || std::fs::read_dir(&stage)
                    .expect("the staging folder lists")
                    .next()
                    .is_none(),
            "a successful commit leaves no staged file either"
        );
        let _ = std::fs::remove_dir_all(&stage);
    }

    /// A manifest that records a data file's length as zero is not believed:
    /// the handle is not told the size, so the file answers for its own
    /// length - one request more - and its rows are read rather than taken
    /// for an empty file's none.
    #[test]
    fn a_manifest_recording_no_length_is_not_believed() {
        let store = super::super::store();
        let root: Folder = super::super::folder(&store, "lake/trades/");
        let schema = schema();
        let spec = PartitionSpec::identity(1, &schema, &["venue"]).expect("venue is a column");
        let mut table = Table::create(
            root.clone(),
            FormatVersion::V2,
            schema.clone(),
            spec.clone(),
        )
        .expect("creates");
        table
            .commit_append(rows(&[1], &["XNAS"]))
            .expect("appends one row");

        // Rewrite the one manifest with the file's length recorded as zero.
        let manifest_key = store
            .keys(BUCKET)
            .into_iter()
            .find(|key| key.ends_with("-m0.avro"))
            .expect("the commit wrote one manifest");
        let relative = manifest_key
            .strip_prefix("lake/trades/")
            .expect("the manifest lives under the table");
        let mut manifest = root
            .child_by_path(relative)
            .expect("a handle on the manifest");
        let mut entries = read_manifest(&manifest).expect("the manifest reads");
        assert_eq!(entries.len(), 1);
        assert!(entries[0].data_file.file_size_in_bytes > 0);
        entries[0].data_file.file_size_in_bytes = 0;
        write_manifest(&mut manifest, FormatVersion::V2, &schema, &spec, &entries)
            .expect("the manifest rewrites");

        let opened = Table::open(root.clone()).expect("opens");
        let files = opened.data_files().expect("the files list");
        assert_eq!(files.len(), 1);
        assert_eq!(
            files[0].0.file_size_in_bytes, 0,
            "the length is recorded as zero"
        );
        let (read, scan) = cost(&store, || drain(opened.scan(None).expect("a scan")));
        assert_eq!(
            read, 1,
            "the file's own row is read, not an empty file's none"
        );
        // The list, the manifest, and the file asked its length before it is
        // read: the one request the recorded length would have saved.
        pin(
            "scan of a file recorded as empty",
            &scan,
            Tally {
                total: 4,
                get: 3,
                head: 1,
                ..Tally::default()
            },
        );
    }
}
