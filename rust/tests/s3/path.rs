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
