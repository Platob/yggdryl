//! `rust/src/s3/folder.rs`: one prefix, or a whole bucket, as a container -
//! its listings, its pages, the stream of the objects beneath it, and what
//! emptying it takes.

mod accounting {
    use yggdryl::IOBase;

    use crate::mod_::{BUCKET, folder, payload, store};
    use crate::server::FakeS3;

    /// Objects under `logs/` at two depths - one without a final newline, one
    /// empty, one hidden - beside one outside the prefix.
    fn logs(store: &FakeS3) {
        for (key, bytes) in [
            ("logs/.hidden", &b"h\n"[..]),
            ("logs/a.log", b"a1\na2\n"),
            ("logs/b.log", b"b1\nb2"),
            ("logs/empty.log", b""),
            ("logs/sub/c.log", b"c1\n"),
            ("other/x.log", b"x1\n"),
        ] {
            store.put(BUCKET, key, bytes);
        }
    }

    /// The methods and keys of the requests recorded since the last clear.
    fn asked(store: &FakeS3) -> Vec<(String, Option<String>)> {
        store
            .requests()
            .into_iter()
            .map(|request| (request.method, request.key))
            .collect()
    }

    /// One `GET` of each object in `keys`.
    fn gets(keys: &[&str]) -> Vec<(String, Option<String>)> {
        keys.iter()
            .map(|key| ("GET".to_owned(), Some((*key).to_owned())))
            .collect()
    }

    #[test]
    fn a_prefix_streams_its_objects_for_one_listing_and_one_get_each() {
        let store = store();
        logs(&store);
        let prefix = folder(&store, "logs/");

        store.clear_requests();
        let stream = prefix.pstream_bytes(0, 4).expect("a stream");
        assert_eq!(store.request_count(), 0, "building the stream asks nothing");

        // Byte order is depth-first pre-order, so `b.log` runs straight into
        // `sub/c.log`; the empty object adds no byte and the hidden one is
        // never fetched.
        let chunks = stream
            .collect::<yggdryl::Result<Vec<_>>>()
            .expect("the stream");
        assert_eq!(chunks.concat(), b"a1\na2\nb1\nb2c1\n");
        assert_eq!(
            chunks.iter().map(Vec::len).collect::<Vec<_>>(),
            [4, 4, 4, 2]
        );
        // One flat listing, then one `GET` per object, each drained whole:
        // the listing already stated every size, so nothing asks for one.
        let mut expected = vec![("GET".to_owned(), None)];
        expected.extend(gets(&[
            "logs/a.log",
            "logs/b.log",
            "logs/empty.log",
            "logs/sub/c.log",
        ]));
        assert_eq!(asked(&store), expected);
        assert!(
            store
                .requests()
                .iter()
                .all(|request| request.method != "HEAD"),
            "no object is asked about before it is read"
        );
    }

    #[test]
    fn a_refused_coded_object_fails_with_the_stores_own_error() {
        let store = store();
        store.put(
            BUCKET,
            "logs/a.log.gz",
            &yggdryl::Codec::Gzip.dump(b"a1\n").expect("an encoding"),
        );
        let prefix = folder(&store, "logs/");

        // The listing answers; the coded object's `GET` is refused. The
        // failure is the store's own, naming the service and the object -
        // not a decoder's complaint about a missing gzip header.
        store.fail_after(1, 403, "AccessDenied", 1);
        let error = prefix.read_all_bytes().expect_err("a refused GET");
        assert!(
            matches!(&error, yggdryl::Error::Remote { status: 403, .. }),
            "{error:?}"
        );
        assert!(error.to_string().contains("logs/a.log.gz"), "{error}");
    }

    #[test]
    fn a_prefix_stream_passes_the_objects_before_its_position_unfetched() {
        let store = store();
        logs(&store);
        let prefix = folder(&store, "logs/");

        // `a.log` is six bytes: a position past it skips it by the size the
        // listing stated, and lands two bytes into `b.log`.
        store.clear_requests();
        let read = prefix
            .pstream_bytes(8, 16)
            .expect("a stream")
            .collect::<yggdryl::Result<Vec<_>>>()
            .expect("the stream")
            .concat();
        assert_eq!(read, b"\nb2c1\n");
        let mut expected = vec![("GET".to_owned(), None)];
        expected.extend(gets(&["logs/b.log", "logs/empty.log", "logs/sub/c.log"]));
        assert_eq!(asked(&store), expected);
        let range = store.requests()[1]
            .headers
            .iter()
            .find(|(name, _)| name == "range")
            .map(|(_, value)| value.clone());
        assert_eq!(range.as_deref(), Some("bytes=2-"), "read from inside");

        // A whole read and a ranged one are the same stream.
        assert_eq!(
            prefix.read_all_bytes().expect("a whole read"),
            b"a1\na2\nb1\nb2c1\n"
        );
        assert_eq!(
            prefix.read_range_bytes(4, 5).expect("a ranged read"),
            b"2\nb1\n"
        );
        assert_eq!(prefix.size(), 0, "a prefix holds no bytes of its own");
    }

    #[test]
    fn a_glob_streams_the_objects_it_matches() {
        let store = store();
        logs(&store);

        // One segment does not descend, `**` does; the hidden object matches
        // neither.
        let level = folder(&store, "logs/*.log");
        store.clear_requests();
        assert_eq!(
            level.read_all_bytes().expect("a whole read"),
            b"a1\na2\nb1\nb2"
        );
        let mut expected = vec![("GET".to_owned(), None)];
        expected.extend(gets(&["logs/a.log", "logs/b.log", "logs/empty.log"]));
        assert_eq!(asked(&store), expected);

        let tree = folder(&store, "logs/**/*.log");
        store.clear_requests();
        assert_eq!(
            tree.read_all_bytes().expect("a whole read"),
            b"a1\na2\nb1\nb2c1\n"
        );
        assert_eq!(
            store.request_count(),
            1 + 4,
            "one listing and one GET per object"
        );
        assert!(
            store
                .requests()
                .iter()
                .all(|request| request.method == "GET"),
            "no HEAD"
        );
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
            .collect::<yggdryl::Result<Vec<_>>>()
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
            .collect::<yggdryl::Result<Vec<_>>>()
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
}

mod protocol {

    use yggdryl::IOBase;

    use crate::mod_::{BUCKET, folder, folder_with, options, store};

    #[test]
    fn a_listing_asks_for_url_encoded_keys_and_a_delimiter_for_one_level() {
        let store = store();
        store.put(BUCKET, "lake/year=2026/part.parquet", b"PAR1");
        store.put(BUCKET, "lake/readme.txt", b"notes");
        let lake = folder(&store, "lake/");

        store.clear_requests();
        let entries: Vec<_> = lake
            .ls(false, false)
            .collect::<yggdryl::Result<Vec<_>>>()
            .expect("a listing");
        let recorded = store.requests();
        let query: std::collections::HashMap<String, String> =
            recorded[0].query.iter().cloned().collect();
        assert_eq!(query.get("list-type").map(String::as_str), Some("2"));
        assert_eq!(query.get("prefix").map(String::as_str), Some("lake/"));
        assert_eq!(query.get("delimiter").map(String::as_str), Some("/"));
        assert_eq!(query.get("encoding-type").map(String::as_str), Some("url"));

        // One level: the sub-prefix rolls up into a container, the key is a leaf.
        assert_eq!(entries.len(), 2);
        let names: Vec<String> = entries
            .iter()
            .filter_map(|entry| entry.url().map(ToString::to_string))
            .collect();
        assert!(
            names.contains(&"s3://trades/lake/readme.txt".to_owned()),
            "{names:?}"
        );
        assert!(
            names.contains(&"s3://trades/lake/year=2026/".to_owned()),
            "{names:?}"
        );

        // A recursive listing drops the delimiter, which is what makes it flat.
        store.clear_requests();
        let _: Vec<_> = lake
            .ls(true, false)
            .collect::<yggdryl::Result<Vec<_>>>()
            .expect("a listing");
        let recorded = store.requests();
        let query: std::collections::HashMap<String, String> =
            recorded[0].query.iter().cloned().collect();
        assert_eq!(query.get("delimiter"), None);
    }

    #[test]
    fn a_listing_pages_through_a_prefix_wider_than_one_page() {
        let store = store();
        for part in 0..25 {
            store.put(BUCKET, &format!("lake/part-{part:03}.parquet"), b"PAR1");
        }
        let lake = folder_with("lake/", options(&store).with_list_page_size(10));

        store.clear_requests();
        let entries: Vec<_> = lake
            .ls(false, false)
            .collect::<yggdryl::Result<Vec<_>>>()
            .expect("a listing");
        assert_eq!(entries.len(), 25);
        assert_eq!(store.request_count(), 3, "ten, ten, and five");

        // The pages are consumed lazily: three entries cost the first page only.
        store.clear_requests();
        let first: Vec<_> = lake.ls(false, false).take(3).collect();
        assert_eq!(first.len(), 3);
        assert_eq!(
            store.request_count(),
            1,
            "a caller taking three pays for one page"
        );
    }

    #[test]
    fn a_prefix_answers_emptiness_from_one_key_and_refuses_to_lose_children() {
        let store = store();
        store.put(BUCKET, "lake/part.parquet", b"PAR1");
        let mut lake = folder(&store, "lake/");

        store.clear_requests();
        assert!(lake.exists());
        assert_eq!(store.request_count(), 1, "one entry settles it");

        // A populated prefix is refused rather than silently recursed.
        let error = lake.remove(false).expect_err("a refusal");
        assert!(error.to_string().contains("still has children"), "{error}");
        assert!(store.get(BUCKET, "lake/part.parquet").is_some());

        // With `recursive` it goes, and the keys with it.
        lake.remove(true).expect("a recursive removal");
        assert!(store.keys(BUCKET).is_empty());
    }

    #[test]
    fn a_glob_descends_its_fixed_prefix_rather_than_the_whole_bucket() {
        let store = store();
        for year in ["2024", "2025", "2026"] {
            for part in 0..2 {
                store.put(
                    BUCKET,
                    &format!("lake/year={year}/part-{part}.parquet"),
                    b"PAR1",
                );
            }
        }
        store.put(BUCKET, "other/ignored.parquet", b"PAR1");
        let lake = folder(&store, "lake/");

        store.clear_requests();
        let matched: Vec<String> = lake
            .glob("year=2025/*.parquet", false)
            .expect("a pattern")
            .collect::<yggdryl::Result<Vec<_>>>()
            .expect("the matches")
            .iter()
            .filter_map(|entry| entry.url().map(ToString::to_string))
            .collect();
        assert_eq!(matched.len(), 2, "{matched:?}");
        assert!(matched.iter().all(|url| url.contains("year=2025")));
        // The fixed prefix was descended, so nothing under `other/` was listed.
        let listed_prefixes: Vec<String> = store
            .requests()
            .iter()
            .filter_map(|request| {
                request
                    .query
                    .iter()
                    .find(|(name, _)| name == "prefix")
                    .map(|(_, value)| value.clone())
            })
            .collect();
        assert!(
            listed_prefixes
                .iter()
                .all(|prefix| prefix.starts_with("lake/")),
            "{listed_prefixes:?}"
        );
    }

    #[test]
    fn a_bucket_lifecycle_this_client_may_not_perform_costs_no_request() {
        let store = store();
        let refusing = || {
            options(&store)
                .with_container_creation(false)
                .with_container_deletion(false)
        };
        let mut ledger =
            yggdryl::s3::folder_with("s3://ledger/", refusing()).expect("a prefix handle");

        store.clear_requests();
        let refused = ledger.create().expect_err("a refusal");
        assert!(
            refused.to_string().contains("create the bucket"),
            "{refused}"
        );
        let refused = ledger.remove(false).expect_err("a refusal");
        assert!(
            refused.to_string().contains("delete the bucket"),
            "{refused}"
        );
        assert_eq!(
            store.request_count(),
            0,
            "the client's own rule is a refusal, not a round trip"
        );
        assert!(!store.buckets().contains(&"ledger".to_owned()));
    }
}
