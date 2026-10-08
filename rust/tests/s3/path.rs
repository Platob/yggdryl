//! `rust/src/s3/path.rs`: one object location, whatever it turns out to be.

mod accounting {
    use yggdryl::{IOBase, IOKind};

    use crate::mod_::{BUCKET, folder, path, store};

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

    /// What each recorded request asked: its method and the key it named, or
    /// `listing` for a request of the bucket.
    fn asked(store: &crate::server::FakeS3) -> Vec<String> {
        store
            .requests()
            .into_iter()
            .map(|request| match request.key {
                Some(key) => format!("{} {key}", request.method),
                None => format!("{} listing", request.method),
            })
            .collect()
    }

    #[test]
    fn the_listing_that_resolves_an_object_states_its_size() {
        let store = store();
        store.put(BUCKET, "lake/part.parquet", b"PAR1....PAR1");

        // The one listing that settles the role states the size beside the
        // key, and the object resolved keeps it: no `HEAD` follows.
        let leaf = path(&store, "lake/part.parquet");
        store.clear_requests();
        assert_eq!(leaf.size(), 12);
        assert_eq!(asked(&store), ["GET listing"], "no HEAD after the listing");
        store.clear_requests();
        assert_eq!(leaf.size(), 12);
        assert_eq!(store.request_count(), 0, "asked again, nothing more");

        // A footer-first read is the object's one suffix-ranged `GET` after it.
        let fresh = path(&store, "lake/part.parquet");
        store.clear_requests();
        assert_eq!(
            fresh.read_tail_bytes(4).expect("the tail"),
            (b"PAR1".to_vec(), 12)
        );
        assert_eq!(asked(&store), ["GET listing", "GET lake/part.parquet"]);
        let range = store.requests()[1]
            .headers
            .iter()
            .find(|(name, _)| name == "range")
            .map(|(_, value)| value.clone());
        assert_eq!(range.as_deref(), Some("bytes=-4"));

        // A write through the location drops what the listing said.
        let mut written = path(&store, "lake/part.parquet");
        assert_eq!(written.size(), 12);
        written.write_all_bytes(b"PAR1").expect("a write");
        store.clear_requests();
        assert_eq!(written.size(), 4);
        assert_eq!(asked(&store), ["HEAD lake/part.parquet"]);

        // Nothing at the location is one listing and no size at all.
        let absent = path(&store, "lake/none.parquet");
        store.clear_requests();
        assert_eq!(absent.size(), 0);
        assert_eq!(absent.read_tail_bytes(4).expect("nothing"), (Vec::new(), 0));
        assert_eq!(asked(&store), ["GET listing"], "the probe is kept");
    }

    /// A Parquet read through a location - what `Holder::from_url` hands a
    /// reader for `s3://bucket/part.parquet` - is the listing that settles
    /// the role and the object's one suffix-ranged `GET`: the listing's size
    /// and the tail's `Content-Range` leave no `HEAD` to send.
    #[cfg(feature = "parquet")]
    #[test]
    fn a_parquet_read_through_a_location_is_its_listing_and_one_tail_get() {
        use std::sync::Arc;

        use arrow_array::{Int64Array, RecordBatch};
        use arrow_schema::{DataType, Field, Schema};
        use yggdryl::IOMedia;

        let store = store();
        let schema = Arc::new(Schema::new(vec![Field::new("id", DataType::Int64, false)]));
        let batch = RecordBatch::try_new(schema, vec![Arc::new(Int64Array::from(vec![1_i64, 2]))])
            .expect("a batch");
        let mut writer = crate::mod_::file(&store, "lake/part.parquet");
        let options = writer.record_options().expect("an encoding");
        writer
            .overwrite_arrow_batch(batch, &options)
            .expect("a written batch");

        let location = path(&store, "lake/part.parquet");
        store.clear_requests();
        let read: usize = location
            .read_arrow_reader(&options)
            .expect("a reader")
            .map(|batch| batch.expect("a batch").num_rows())
            .sum();
        assert_eq!(read, 2);
        assert_eq!(asked(&store), ["GET listing", "GET lake/part.parquet"]);
        let range = store.requests()[1]
            .headers
            .iter()
            .find(|(name, _)| name == "range")
            .map(|(_, value)| value.clone());
        assert_eq!(range.as_deref(), Some("bytes=-1048576"));
    }

    /// Two objects under `logs/`, one a level deeper and one without a final
    /// newline, beside a hidden one and one outside the prefix.
    fn logs(store: &crate::server::FakeS3) {
        store.put(BUCKET, "logs/.hidden", b"h\n");
        store.put(BUCKET, "logs/a.log", b"a1\na2");
        store.put(BUCKET, "logs/sub/b.log", b"b1\n");
        store.put(BUCKET, "other.log", b"o\n");
    }

    /// The objects a stream from `handle` reads, and what reading them cost.
    fn streamed(store: &crate::server::FakeS3, handle: &dyn IOBase) -> (Vec<u8>, Vec<String>) {
        store.clear_requests();
        let bytes = handle
            .pstream_bytes(0, 3)
            .expect("a stream")
            .collect::<yggdryl::Result<Vec<_>>>()
            .expect("the stream")
            .concat();
        let asked = store
            .requests()
            .into_iter()
            .map(|request| match request.key {
                Some(key) => format!("{} {key}", request.method),
                None => format!("{} listing", request.method),
            })
            .collect();
        (bytes, asked)
    }

    #[test]
    fn a_location_spelled_as_a_container_streams_its_objects() {
        let store = store();
        logs(&store);

        // The slash settles the role without a probe, so the stream costs the
        // one listing and one `GET` per object, and never a `HEAD`.
        let (bytes, asked) = streamed(&store, &path(&store, "logs/"));
        assert_eq!(bytes, b"a1\na2b1\n");
        assert_eq!(
            asked,
            ["GET listing", "GET logs/a.log", "GET logs/sub/b.log"]
        );

        // A glob says it too: one segment matches one level and does not
        // descend, and `**` does.
        let (bytes, asked) = streamed(&store, &path(&store, "logs/*.log"));
        assert_eq!(bytes, b"a1\na2");
        assert_eq!(asked, ["GET listing", "GET logs/a.log"]);
        let (bytes, asked) = streamed(&store, &path(&store, "logs/**/*.log"));
        assert_eq!(bytes, b"a1\na2b1\n");
        assert_eq!(
            asked,
            ["GET listing", "GET logs/a.log", "GET logs/sub/b.log"]
        );

        // Reading the whole location and a range of it read the same stream.
        let location = path(&store, "logs/");
        assert_eq!(
            location.read_all_bytes().expect("a whole read"),
            b"a1\na2b1\n"
        );
        assert_eq!(
            location.read_range_bytes(3, 3).expect("a ranged read"),
            b"a2b"
        );
        assert_eq!(location.size(), 0, "a prefix holds no bytes of its own");
    }

    #[test]
    fn a_plain_name_that_turns_out_to_be_a_prefix_streams_its_objects() {
        let store = store();
        logs(&store);

        // The name alone does not say, so one probe settles it first.
        let (bytes, asked) = streamed(&store, &path(&store, "logs"));
        assert_eq!(bytes, b"a1\na2b1\n");
        assert_eq!(
            asked,
            [
                "GET listing",
                "GET listing",
                "GET logs/a.log",
                "GET logs/sub/b.log"
            ]
        );

        // And a name that turns out to be an object streams that object, for
        // the same probe.
        let (bytes, asked) = streamed(&store, &path(&store, "logs/a.log"));
        assert_eq!(bytes, b"a1\na2");
        assert_eq!(asked, ["GET listing", "GET logs/a.log"]);
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
    fn whether_a_spelled_container_is_there_is_one_listing() {
        let store = store();

        // The spelling settles the role for nothing; whether anything is
        // there is asked all the same, as one listing of one key, and an
        // empty prefix is not there.
        let spelled = path(&store, "lake/");
        store.clear_requests();
        assert_eq!(spelled.kind(), IOKind::Directory);
        assert_eq!(store.request_count(), 0, "the spelling settled the role");
        assert!(!spelled.exists());
        assert_eq!(store.request_count(), 1, "one listing of one key");

        store.put(BUCKET, "lake/part.parquet", b"PAR1");
        let populated = path(&store, "lake/");
        store.clear_requests();
        assert!(populated.exists());
        assert_eq!(store.request_count(), 1);

        // The bucket is asked with one `HEAD`.
        let bucket = path(&store, "");
        store.clear_requests();
        assert!(bucket.exists());
        assert_eq!(store.request_count(), 1);
        assert_eq!(store.requests()[0].method, "HEAD");

        // A glob is its listing up to the first match: one page here,
        // whichever way it answers.
        logs(&store);
        let matching = path(&store, "logs/*.log");
        store.clear_requests();
        assert!(matching.exists());
        assert_eq!(store.request_count(), 1, "one listing");
        assert_eq!(store.requests()[0].method, "GET");
        let none = path(&store, "logs/*.csv");
        store.clear_requests();
        assert!(!none.exists());
        assert_eq!(store.request_count(), 1, "one page, read to its end");
    }
}

mod protocol {

    use yggdryl::{Error, IOBase, IOKind};

    use crate::mod_::{BUCKET, folder, options, path, store};

    #[test]
    fn keys_that_a_url_cannot_spell_survive_the_round_trip() {
        let store = store();
        // Spaces, a plus, an equals, a percent, and a unicode name: every one of
        // them means something different to a URL than to a key.
        let awkward = [
            "lake/a b/c.parquet",
            "lake/a+b/c.parquet",
            "lake/year=2026/part.parquet",
            "lake/100%/done.parquet",
            "lake/données/prix.parquet",
        ];
        for key in awkward {
            store.put(BUCKET, key, key.as_bytes());
        }

        let lake = folder(&store, "lake/");
        let found: Vec<String> = lake
            .ls(true, false)
            .collect::<yggdryl::Result<Vec<_>>>()
            .expect("a listing")
            .iter()
            .filter(|entry| !entry.is_container())
            .map(|entry| {
                // Reading each listed entry addresses the same object it names.
                let bytes = entry.read_all_bytes().expect("the object");
                String::from_utf8(bytes).expect("the key it stores")
            })
            .collect();
        for key in awkward {
            assert!(
                found.contains(&key.to_owned()),
                "{key} missing from {found:?}"
            );
        }
    }

    #[test]
    fn a_location_naming_no_container_is_refused_before_anything_is_built() {
        let error = yggdryl::s3::file("file:///tmp/part.parquet").expect_err("a refusal");
        assert!(error.to_string().contains("naming a container"), "{error}");
        // The credential and endpoint knobs are equally unusable without one.
        yggdryl::s3::folder("https://example.com/x").expect_err("a refusal");
        // A store's own word for it is what its own refusal uses: a location that
        // names only an endpoint has named no container yet.
        let error = yggdryl::s3::file("gs://storage.googleapis.com/").expect_err("a refusal");
        assert!(error.to_string().contains("naming a bucket"), "{error}");
        let error = yggdryl::s3::file("az://trades.blob.core.windows.net/").expect_err("a refusal");
        assert!(error.to_string().contains("naming a container"), "{error}");
    }

    #[test]
    fn every_s3_url_spelling_reaches_the_same_object() {
        // `s3`, `s3a`, and `s3n` name one protocol - the Hadoop spellings differ
        // only in the connector that once read them - so all three have to reach
        // this backend and address the same key, not just parse.
        let store = store();
        for scheme in ["s3", "s3a", "s3n"] {
            let url = format!("{scheme}://{BUCKET}/lake/{scheme}.bin");
            let mut handle =
                yggdryl::s3::file_with(&url, options(&store)).expect("an object handle");
            handle.write_all_bytes(b"AAPL").expect("a write");

            assert_eq!(handle.bucket(), BUCKET);
            assert_eq!(handle.key(), format!("lake/{scheme}.bin"));
            // The spelling the caller used is the one the handle reports back, so
            // a location survives a round trip through a listing or a log.
            assert_eq!(handle.url().to_string(), url);
            assert_eq!(
                store.get(BUCKET, &format!("lake/{scheme}.bin")),
                Some(b"AAPL".to_vec())
            );

            // A prefix and a location resolve over the same store just as well.
            let prefix = format!("{scheme}://{BUCKET}/lake/");
            let listed = yggdryl::s3::folder_with(&prefix, options(&store))
                .expect("a prefix handle")
                .ls(false, false)
                .filter_map(|entry| entry.ok())
                .filter_map(|entry| entry.url().map(ToString::to_string))
                .collect::<Vec<_>>();
            assert!(listed.contains(&url), "{listed:?}");
            assert_eq!(
                yggdryl::s3::located_with(&url, options(&store))
                    .expect("a location")
                    .kind(),
                IOKind::File
            );
        }

        // A refusal names the location the handle reports, spelling included, so
        // an error text is something a caller can paste back as a location.
        store.require_access_key(Some("SOMEONEELSE"));
        for scheme in ["s3", "s3a", "s3n"] {
            let url = format!("{scheme}://{BUCKET}/lake/{scheme}.bin");
            let refused = yggdryl::s3::file_with(&url, options(&store))
                .expect("an object handle")
                .read_all_bytes()
                .expect_err("a refusal");
            match &refused {
                Error::Remote { path, status, .. } => {
                    assert_eq!(*status, 403);
                    assert_eq!(path.as_str(), url);
                }
                other => panic!("expected a remote refusal, got {other:?}"),
            }
        }
    }

    #[test]
    fn a_path_that_shares_a_textual_prefix_is_not_mistaken_for_a_container() {
        let store = store();
        store.put(BUCKET, "lake/partial.parquet", b"PAR1");
        // `lake/part` is neither an object nor a prefix, even though a key starts
        // with those very characters.
        let ambiguous = path(&store, "lake/part");
        assert_eq!(ambiguous.kind(), IOKind::Unknown);

        // Adding the object settles it as a leaf...
        store.put(BUCKET, "lake/part", b"AAPL");
        assert_eq!(path(&store, "lake/part").kind(), IOKind::File);
        // ...and a key beneath it makes the same name a container as well, where
        // the exact object wins because it sorts first.
        store.put(BUCKET, "lake/part/under.parquet", b"PAR1");
        assert_eq!(path(&store, "lake/part").kind(), IOKind::File);
    }
}
