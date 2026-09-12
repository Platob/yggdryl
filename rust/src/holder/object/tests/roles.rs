//! The shared contracts, exercised over S3 rather than restated for it.
//!
//! The point of a backend supplying the three roles is that everything built
//! on [`IOBase`](crate::IOBase) works over it unchanged. These tests reach for
//! the derived surfaces - wrappers, codings, record media, partitions - and
//! check they behave over a store exactly as they do over a mapped file.

use super::{BUCKET, file, folder, path, payload, store};
use crate::holder::buffered::BufferedOptions;
use crate::{IOBase, IOKind};

#[test]
fn the_byte_contract_holds_over_a_store() {
    let store = store();
    let mut handle = file(&store, "lake/part.bin");

    // Writing past the end grows the value and zero-fills the gap.
    handle.pwrite(0, b"trade").expect("a write");
    handle.pwrite(8, b"!").expect("a write");
    handle.flush().expect("a publish");
    assert_eq!(handle.read_all_bytes().expect("the value"), b"trade\0\0\0!");
    assert_eq!(handle.size(), 9);

    // Truncating shrinks and extends, zero-filling rather than leaving stale
    // bytes visible.
    handle.truncate(4).expect("a truncation");
    handle.flush().expect("a publish");
    assert_eq!(handle.read_all_bytes().expect("the value"), b"trad");
    handle.truncate(6).expect("a truncation");
    handle.flush().expect("a publish");
    assert_eq!(handle.read_all_bytes().expect("the value"), b"trad\0\0");

    // Capacity is never below size, and an exact read names its shortfall.
    assert!(handle.capacity() >= handle.size());
    let message = handle
        .pread_exact(0, &mut [0_u8; 32])
        .expect_err("a shortfall")
        .to_string();
    assert!(message.contains("expected 32 bytes"), "{message}");

    // Clearing empties the object and keeps it.
    handle.clear().expect("an emptied object");
    assert_eq!(handle.size(), 0);
    assert!(handle.is_empty());
}

#[test]
fn a_page_cache_over_an_opened_store_handle_fetches_one_page_per_miss() {
    let store = store();
    let bytes = payload(8192);
    store.put(BUCKET, "lake/part.parquet", &bytes);
    let mut handle = file(&store, "lake/part.parquet");
    // Opening is what a scan does, and it is what keeps the metadata the
    // cache consults from being a round trip of its own.
    handle.open().expect("an open");
    let handle = handle.buffered(BufferedOptions::default().with_page_size(1024));

    store.clear_requests();
    assert_eq!(
        handle.read_range_bytes(0, 16).expect("a header"),
        &bytes[..16]
    );
    let first = store.request_count();
    assert_eq!(first, 1, "the miss fetched exactly one page");
    assert_eq!(
        store.requests()[0]
            .headers
            .iter()
            .find(|(name, _)| name == "range")
            .map(|(_, value)| value.as_str()),
        Some("bytes=0-1023"),
        "one page, not the object"
    );

    // The same bytes again, and the rest of that page, reach no further than
    // memory: the wrapper works unchanged over a remote handle.
    assert_eq!(
        handle.read_range_bytes(0, 16).expect("a header"),
        &bytes[..16]
    );
    assert_eq!(
        handle.read_range_bytes(16, 64).expect("more"),
        &bytes[16..80]
    );
    assert_eq!(store.request_count(), first, "both hits were free");
    assert_eq!(handle.cached_pages(), 1);

    // A second page is one more fetch, and the first stays cached.
    assert_eq!(
        handle
            .read_range_bytes(2048, 8)
            .expect("a later page")
            .len(),
        8
    );
    assert_eq!(store.request_count(), first + 1);
    assert_eq!(handle.cached_pages(), 2);
}

#[test]
fn a_closed_handle_answers_metadata_freshly_every_time_it_is_asked() {
    let store = store();
    store.put(BUCKET, "lake/part.parquet", &payload(1024));
    let handle = file(&store, "lake/part.parquet");

    // This is the open/close contract, not an oversight: a closed handle
    // holds nothing, so each question is a `HEAD`. It is why a scan opens.
    store.clear_requests();
    assert_eq!(handle.size(), 1024);
    assert_eq!(handle.size(), 1024);
    assert_eq!(store.request_count(), 2);
    assert!(
        store
            .requests()
            .iter()
            .all(|request| request.method == "HEAD"),
        "metadata questions never transfer bytes"
    );
}

#[test]
fn a_content_coding_is_read_and_written_in_place_over_a_store() {
    let store = store();
    let mut handle = file(&store, "lake/quotes.json.gz");
    // The name declares the coding, so the handle encodes on the way out and
    // decodes on the way in without a caller naming a codec.
    assert_eq!(handle.codec(), crate::Codec::Gzip);

    let value =
        crate::Scalar::from_record([("symbol", crate::Scalar::from("AAPL"))]).expect("a record");
    handle.write_scalar(&value).expect("a compressed write");
    assert_eq!(handle.read_scalar(None).expect("the value"), value);

    // What is stored is the compressed form, not the JSON.
    let stored = store
        .get(BUCKET, "lake/quotes.json.gz")
        .expect("the object");
    assert_eq!(&stored[..2], &[0x1f, 0x8b], "a gzip member");
}

#[cfg(feature = "arrow")]
#[test]
fn records_round_trip_through_a_store_like_any_other_handle() {
    use crate::IOMedia;
    use std::sync::Arc;

    let store = store();
    let field = crate::DataType::from_fields([
        crate::DataType::Int64.required_field("id"),
        crate::DataType::utf8().required_field("symbol"),
    ])
    .expect("a struct root")
    .required_field("row");
    let batch = arrow_array::RecordBatch::try_new(
        field.clone().into_arrow_schema().expect("a schema"),
        vec![
            Arc::new(arrow_array::Int64Array::from(vec![1_i64, 2])),
            Arc::new(arrow_array::StringArray::from(vec!["AAPL", "MSFT"])),
        ],
    )
    .expect("a batch");

    let mut handle = file(&store, "lake/part.arrows");
    let options = handle.record_options().expect("an encoding");
    handle
        .overwrite_arrow_batch(batch.clone(), &options)
        .expect("a written batch");

    let read: Vec<_> = handle
        .read_arrow_reader(&options)
        .expect("a reader")
        .collect::<std::result::Result<Vec<_>, _>>()
        .expect("every batch");
    assert_eq!(read.len(), 1);
    assert_eq!(read[0], batch);
    assert_eq!(handle.row_size().expect("a row count"), 2);
    assert_eq!(handle.column_size().expect("a column count"), 2);
}

#[test]
fn hive_partitions_are_read_off_a_key_and_select_the_leaves_carrying_them() {
    let store = store();
    for year in ["2025", "2026"] {
        for part in 0..2 {
            store.put(
                BUCKET,
                &format!("lake/year={year}/part-{part}.parquet"),
                b"PAR1",
            );
        }
    }
    let lake = folder(&store, "lake/");

    let leaf = lake
        .child_by_path("year=2026/part-0.parquet")
        .expect("a child");
    assert_eq!(
        leaf.partitions(),
        vec![("year".to_owned(), "2026".to_owned())]
    );

    let selected: Vec<String> = lake
        .children_where(&[("year", "2026")], false)
        .expect("a partition filter")
        .collect::<crate::Result<Vec<_>>>()
        .expect("the leaves")
        .iter()
        .filter_map(|entry| entry.url().map(ToString::to_string))
        .collect();
    assert_eq!(selected.len(), 2, "{selected:?}");
    assert!(selected.iter().all(|url| url.contains("year=2026")));
}

#[test]
fn a_listing_skips_private_names_unless_they_are_asked_for() {
    let store = store();
    store.put(BUCKET, "lake/part.parquet", b"PAR1");
    store.put(BUCKET, "lake/.hidden", b"x");
    store.put(BUCKET, "lake/.staging/part.parquet", b"PAR1");
    let lake = folder(&store, "lake/");

    assert_eq!(lake.ls(false, false).count(), 1, "the visible leaf alone");
    assert_eq!(
        lake.ls(false, true).count(),
        3,
        "the leaf, the file, the prefix"
    );
    // A private prefix is not descended into either.
    assert_eq!(lake.ls(true, false).count(), 1);
    assert_eq!(lake.ls(true, true).count(), 4);
}

#[test]
fn a_generic_location_writes_itself_into_being_as_an_object() {
    let store = store();
    let mut location = path(&store, "lake/part.bin");
    assert_eq!(location.kind(), IOKind::Unknown, "nothing has decided yet");

    // A byte write is what settles an undecided location.
    location.write_all_bytes(b"AAPL").expect("a write");
    assert_eq!(location.read_all_bytes().expect("the value"), b"AAPL");
    assert_eq!(
        store.get(BUCKET, "lake/part.bin").expect("the object"),
        b"AAPL"
    );

    // A location spelled as a container stays one, and truncating it to zero
    // is how a bucket root would be brought into being.
    let spelled = path(&store, "lake/");
    assert!(spelled.is_container());
    assert_eq!(spelled.ls(false, false).count(), 1);
}

#[test]
fn a_holder_walks_a_store_through_one_type() {
    let store = store();
    store.put(BUCKET, "lake/year=2026/part.parquet", b"PAR1");
    let root = crate::holder::Holder::ObjectFolder(folder(&store, ""));

    assert!(root.is_container());
    assert_eq!(root.kind(), IOKind::Directory);
    let entries: Vec<_> = root
        .ls(true, false)
        .collect::<crate::Result<Vec<_>>>()
        .expect("a listing");
    assert_eq!(entries.len(), 3, "two containers and the leaf");
    let leaf = entries.last().expect("the leaf");
    assert_eq!(leaf.read_all_bytes().expect("the object"), b"PAR1");
    assert!(matches!(leaf, crate::holder::Holder::ObjectFile(_)));

    // Resolving down the tree stays in one type the whole way.
    let child = root
        .child_by_path("lake/year=2026/part.parquet")
        .expect("a child");
    assert_eq!(child.read_all_bytes().expect("the object"), b"PAR1");
}

#[test]
fn a_bucket_root_is_a_container_that_creates_and_removes_the_bucket() {
    let store = super::FakeS3::start();
    let client = std::sync::Arc::new(
        super::Client::new(
            &crate::Url::from_str("s3://fresh/").expect("a location"),
            super::options(&store),
        )
        .expect("a client"),
    );
    let mut root = super::Folder::new(
        client,
        crate::Url::from_str("s3://fresh/").expect("a location"),
    )
    .expect("a bucket handle");

    assert!(!root.exists());
    root.create().expect("a created bucket");
    assert!(store.buckets().contains(&"fresh".to_owned()));
    assert!(root.exists());
    assert_eq!(root.kind(), IOKind::Directory);
    assert_eq!(root.size(), 0);

    root.remove(true).expect("a removed bucket");
    assert!(!store.buckets().contains(&"fresh".to_owned()));
}

#[test]
fn a_cursor_over_a_store_keeps_its_own_position() {
    let store = store();
    store.put(BUCKET, "lake/part.bin", b"0123456789");
    let mut cursor = file(&store, "lake/part.bin").cursor_at(2);

    use crate::IOCursor;
    let first = cursor
        .stream_bytes(3)
        .expect("a stream")
        .next()
        .transpose()
        .expect("a chunk")
        .expect("some bytes");
    assert_eq!(first, b"234");
    assert_eq!(cursor.tell(), 5);
}

#[test]
fn a_container_reads_as_the_table_beneath_it() {
    let store = store();
    store.put(BUCKET, "lake/year=2026/part.parquet", b"PAR1");
    let lake = folder(&store, "lake/");

    // A folder of tabular leaves is tabular; it holds no bytes of its own.
    assert!(lake.is_tabular());
    assert!(!lake.is_atomic());
    assert!(lake.is_io());
    assert_eq!(lake.size(), 0);
    assert_eq!(lake.read_all_bytes().expect("no bytes").len(), 0);
    // Writing bytes to a container is refused rather than silently accepted.
    let mut lake = lake;
    let error = lake.pwrite(0, b"x").expect_err("a refusal");
    assert!(error.to_string().contains("directory"), "{error}");
}

#[test]
fn a_directory_marker_lists_once_as_the_container_it_names() {
    let store = store();
    // What a console or an older tool writes to make a prefix visible.
    store.put(BUCKET, "lake/year=2026/", b"");
    store.put(BUCKET, "lake/year=2026/part.parquet", b"PAR1");
    let lake = folder(&store, "lake/");

    let level: Vec<(String, bool)> = lake
        .ls(false, false)
        .map(|entry| entry.expect("an entry"))
        .map(|entry| {
            (
                entry.url().expect("a location").to_string(),
                entry.is_container(),
            )
        })
        .collect();
    assert_eq!(
        level,
        vec![("s3://trades/lake/year=2026/".to_owned(), true)],
        "one level, one container"
    );

    let subtree: Vec<(String, bool)> = lake
        .ls(true, false)
        .map(|entry| entry.expect("an entry"))
        .map(|entry| {
            (
                entry.url().expect("a location").to_string(),
                entry.is_container(),
            )
        })
        .collect();
    assert_eq!(
        subtree,
        vec![
            ("s3://trades/lake/year=2026/".to_owned(), true),
            ("s3://trades/lake/year=2026/part.parquet".to_owned(), false),
        ],
        "the marker is the container, not a leaf beside it and not a second copy"
    );
}

#[test]
fn a_closed_handle_reads_what_is_there_now_rather_than_what_a_listing_saw() {
    let store = store();
    store.put(BUCKET, "lake/part.bin", &payload(16));
    let listed = folder(&store, "lake/")
        .ls(false, false)
        .next()
        .expect("an entry")
        .expect("an entry");
    assert!(!listed.opened(), "a listing opens nothing");
    // A listing states the size, so asking for it costs nothing.
    assert_eq!(listed.size(), 16);

    // The object grows out of band, which is the ordinary case on a store.
    store.put(BUCKET, "lake/part.bin", &payload(4096));
    let window = listed.read_range_bytes(1000, 8).expect("a read");
    assert_eq!(
        window,
        payload(4096)[1000..1008],
        "a size a listing reported bounds nothing on a closed handle"
    );
}

#[test]
fn a_read_longer_than_the_object_allocates_the_object() {
    let store = store();
    store.put(BUCKET, "lake/part.bin", &payload(16));
    let handle = file(&store, "lake/part.bin");
    // Asking for the rest of an object whose length is unknown is ordinary,
    // and the store's answer is what says how much there is to hold.
    let bytes = handle
        .read_range_bytes(0, 8 * 1024 * 1024 * 1024)
        .expect("a read");
    assert_eq!(bytes, payload(16));
    assert_eq!(store.request_count(), 1, "still one ranged GET");
}

#[test]
fn a_staged_write_is_what_the_location_streams_and_never_what_it_deletes() {
    let store = store();
    let mut handle = path(&store, "lake/part.csv");
    handle.pwrite(0, b"AAPL,187.23\n").expect("a staged write");

    let streamed: Vec<u8> = handle
        .pstream_bytes(0, 4)
        .expect("a stream")
        .flat_map(|chunk| chunk.expect("bytes"))
        .collect();
    assert_eq!(
        streamed, b"AAPL,187.23\n",
        "a caller reads what it just wrote, published or not"
    );

    // Removing a location with a write still pending must not publish it on
    // the way past: the delete would race its own resurrection.
    handle.remove(false).expect("a removal");
    drop(handle);
    assert!(
        store.keys(BUCKET).is_empty(),
        "the staged write was abandoned, not published: {:?}",
        store.keys(BUCKET)
    );
}

#[test]
fn reserving_space_creates_nothing_because_it_changes_no_length() {
    let store = store();
    {
        let mut handle = file(&store, "lake/part.bin");
        handle.reserve(4096).expect("a reservation");
        assert_eq!(handle.size(), 0, "reserving never changes a length");
    }
    assert!(
        store.keys(BUCKET).is_empty(),
        "a hint about an allocation is not a write: {:?}",
        store.keys(BUCKET)
    );
}
