//! The same operations, spoken to each of the three stores.
//!
//! One fake store answers all three dialects over one set of objects, so these
//! tests do two things at once: they exercise each store's own wire shape, and
//! they check that the three agree on what a key, a prefix, and a listing are.
//! A value written through Google's JSON API and read back through Azure's REST
//! API is the same object because it is the same object - which is exactly what
//! a caller who moves a lake between clouds is relying on.

use crate::holder::object::Provider;
use crate::{IOBase, IOFolder, IOKind};

use super::{BUCKET, file_on, folder_on, options_for, payload, store};

/// Every store, so a test that has nothing store-specific to say runs on all
/// three rather than on whichever one was written first.
const EVERY: [Provider; 3] = [Provider::Aws, Provider::Google, Provider::Azure];

#[test]
fn a_value_written_to_any_store_reads_back_from_it_whole_and_by_range() {
    let store = store();
    for provider in EVERY {
        let key = format!("lake/{}.bin", provider.service());
        let mut handle = file_on(&store, provider, &key);
        handle.write_all_bytes(b"AAPL,187.23").expect("a write");

        assert_eq!(handle.read_all_bytes().expect("a read"), b"AAPL,187.23");
        assert_eq!(handle.size(), 11);
        assert_eq!(
            handle.read_range_bytes(5, 6).expect("a ranged read"),
            b"187.23",
            "{provider}"
        );
        assert!(handle.exists(), "{provider}");
    }
}

#[test]
fn a_value_written_through_one_store_is_read_through_another() {
    // The three name the same object, so the only thing that can differ is the
    // dialect the request was written in.
    let store = store();
    let mut written = file_on(&store, Provider::Google, "lake/shared.bin");
    written.write_all_bytes(b"one object").expect("a write");

    for provider in [Provider::Aws, Provider::Azure] {
        let read = file_on(&store, provider, "lake/shared.bin");
        assert_eq!(
            read.read_all_bytes().expect("a read"),
            b"one object",
            "{provider}"
        );
    }
}

#[test]
fn a_listing_rolls_prefixes_up_the_same_way_on_every_store() {
    let store = store();
    for key in [
        "lake/year=2026/a.parquet",
        "lake/year=2026/b.parquet",
        "lake/year=2027/c.parquet",
        "lake/root.parquet",
    ] {
        file_on(&store, Provider::Aws, key)
            .write_all_bytes(b"x")
            .expect("a write");
    }
    for provider in EVERY {
        let lake = folder_on(&store, provider, "lake/");
        let mut named: Vec<String> = lake
            .ls(false, false)
            .filter_map(Result::ok)
            .filter_map(|entry| entry.url().map(|url| url.to_string()))
            .collect();
        named.sort();
        let scheme = provider.scheme();
        let scheme = scheme.as_str();
        assert_eq!(
            named,
            [
                format!("{scheme}://{BUCKET}/lake/root.parquet"),
                format!("{scheme}://{BUCKET}/lake/year=2026/"),
                format!("{scheme}://{BUCKET}/lake/year=2027/"),
            ],
            "{provider}"
        );
    }
}

#[test]
fn a_recursive_listing_is_one_flat_read_on_every_store() {
    let store = store();
    for index in 0..12 {
        file_on(&store, Provider::Aws, &format!("lake/part-{index:02}.bin"))
            .write_all_bytes(b"x")
            .expect("a write");
    }
    for provider in EVERY {
        let lake = folder_on(&store, provider, "lake/");
        let listed = lake.ls(true, false).filter_map(Result::ok).count();
        assert_eq!(listed, 12, "{provider}");
        assert_eq!(
            lake.stats().lists,
            1,
            "one listing answers the whole subtree on {provider}"
        );
    }
}

#[test]
fn a_large_value_is_uploaded_in_chunks_and_assembles_to_what_was_written() {
    // Three protocols for one idea: numbered parts, a resumable session, and
    // staged blocks. What each has to end up with is the same bytes.
    let store = store();
    let bytes = payload(9 * 1024 * 1024);
    for provider in EVERY {
        let key = format!("lake/{}-big.bin", provider.service());
        let url = super::location_on(provider, &key);
        let client = std::sync::Arc::new(
            crate::holder::object::client::Client::new(
                &url,
                options_for(&store, provider)
                    .with_part_size(5 * 1024 * 1024)
                    .with_multipart_threshold(1024 * 1024),
            )
            .expect("a client"),
        );
        let mut handle = crate::holder::object::File::new(client, url).expect("an object handle");
        handle.write_all_bytes(&bytes).expect("a write");
        assert_eq!(handle.size(), bytes.len() as u64, "{provider}");
        assert_eq!(
            handle.read_all_bytes().expect("a read"),
            bytes,
            "{provider}"
        );
    }
}

#[test]
fn removing_a_prefix_is_one_listing_and_one_batch_on_every_store() {
    let store = store();
    for provider in EVERY {
        let prefix = format!("{}-scratch/", provider.service());
        for index in 0..5 {
            file_on(&store, provider, &format!("{prefix}part-{index}.bin"))
                .write_all_bytes(b"x")
                .expect("a write");
        }
        let mut folder = folder_on(&store, provider, &prefix);
        folder.folder_remove(true).expect("a removal");
        let stats = folder.stats();
        assert_eq!(
            folder_on(&store, provider, &prefix)
                .ls(true, false)
                .filter_map(Result::ok)
                .count(),
            0,
            "{provider}"
        );
        assert_eq!(stats.lists, 1, "one listing on {provider}");
        assert_eq!(
            stats.posts + stats.deletes,
            1,
            "one bulk delete on {provider}"
        );
    }
}

#[test]
fn a_container_is_created_and_removed_through_every_store() {
    let store = store();
    for provider in EVERY {
        let name = format!("made-by-{}", provider.service());
        let url = crate::Url::from_str(&format!("{}://{name}/", provider.scheme().as_str()))
            .expect("a location");
        let client = std::sync::Arc::new(
            crate::holder::object::client::Client::new(&url, options_for(&store, provider))
                .expect("a client"),
        );
        let mut folder =
            crate::holder::object::Folder::new(client, url).expect("a container handle");
        assert!(!folder.folder_exists(), "{provider}");
        folder.create_folder().expect("a creation");
        assert!(folder.folder_exists(), "{provider}");
        folder.folder_remove(false).expect("a removal");
        assert!(!folder.folder_exists(), "{provider}");
    }
}

#[test]
fn a_location_resolves_to_what_it_names_on_every_store() {
    let store = store();
    file_on(&store, Provider::Aws, "lake/part.parquet")
        .write_all_bytes(b"PAR1")
        .expect("a write");
    for provider in EVERY {
        let url = super::location_on(provider, "lake/part.parquet");
        let client = std::sync::Arc::new(
            crate::holder::object::client::Client::new(&url, options_for(&store, provider))
                .expect("a client"),
        );
        let leaf = crate::holder::object::Path::new(client, url).expect("a location");
        assert_eq!(leaf.kind(), IOKind::File, "{provider}");

        // A fresh client, so the counters describe this resolution alone.
        let url = super::location_on(provider, "lake/");
        let client = std::sync::Arc::new(
            crate::holder::object::client::Client::new(&url, options_for(&store, provider))
                .expect("a client"),
        );
        let prefix = crate::holder::object::Path::new(client, url).expect("a location");
        assert_eq!(prefix.kind(), IOKind::Directory, "{provider}");
        assert_eq!(
            prefix.stats().requests,
            0,
            "a trailing slash already said it is a container on {provider}"
        );
    }
}

#[test]
fn a_missing_object_reads_empty_and_a_missing_container_is_named_in_the_refusal() {
    let store = store();
    for provider in EVERY {
        let handle = file_on(&store, provider, "lake/never-written.bin");
        assert_eq!(handle.read_all_bytes().expect("a read"), b"", "{provider}");
        assert_eq!(handle.size(), 0, "{provider}");
        assert!(!handle.exists(), "{provider}");
    }
}
