//! `rust/src/object/mod.rs`: the fixtures every object-store suite builds on.
//!
//! Every test here runs against [`server::FakeS3`], an in-process store that
//! speaks all three dialects - Amazon S3, Google's JSON API, Azure Blob
//! Storage - over one set of objects, and records each request. That is what
//! lets the suite assert the thing this backend is designed for - the *number
//! of round trips* an operation costs - rather than only that it produced the
//! right bytes. A change that makes a read cost two requests instead of one is
//! a failing test, not a silent regression.
//!
//! One store answering three dialects is also what lets a test write with one
//! and read with another, which is the cheapest possible check that the three
//! agree on what a key and a prefix are.
//!
//! Every handle below is built through the entry points
//! `rust/src/object/mod.rs` publishes - [`object::file_with`],
//! [`object::folder_with`], [`object::located_with`] - so the fixtures are
//! themselves the pin on the crate's own door: a location a suite spells is
//! the location a handle reports.

#![allow(dead_code)]

use yggdryl::object::{
    self, AzureOptions, Credentials, GoogleOptions, ObjectFile, ObjectFolder, ObjectOptions,
    ObjectPath, Provider,
};

use crate::server::{self, FakeS3};

/// The bucket every fixture writes into.
pub const BUCKET: &str = "trades";

/// A running fake store with `bucket` created.
pub fn store() -> FakeS3 {
    let store = FakeS3::start();
    store.create_bucket(BUCKET);
    store
}

/// Options that reach `store` and consult nothing outside the test.
pub fn options(store: &FakeS3) -> ObjectOptions {
    ObjectOptions::default()
        .with_environment(false)
        .with_endpoint(store.endpoint())
        .with_region("us-east-1")
        .with_path_style(true)
        .with_credentials(Credentials::new("AKIAIOSFODNN7EXAMPLE", "wJalrXUtnFEMI"))
}

/// The object `key` on `store`.
pub fn file(store: &FakeS3, key: &str) -> ObjectFile {
    file_with(key, options(store))
}

/// The object `key`, under options a test tightened.
///
/// The options already name the endpoint, so the store is not passed again.
pub fn file_with(key: &str, options: ObjectOptions) -> ObjectFile {
    object::file_with(&location(key), options).expect("an object handle")
}

/// The prefix `key` on `store`.
pub fn folder(store: &FakeS3, key: &str) -> ObjectFolder {
    folder_with(key, options(store))
}

/// The prefix `key`, under options a test tightened.
pub fn folder_with(key: &str, options: ObjectOptions) -> ObjectFolder {
    object::folder_with(&location(key), options).expect("a prefix handle")
}

/// The location `key` on `store`.
pub fn path(store: &FakeS3, key: &str) -> ObjectPath {
    path_with(key, options(store))
}

/// The location `key`, under options a test tightened.
pub fn path_with(key: &str, options: ObjectOptions) -> ObjectPath {
    object::path_at_with(Provider::Aws, BUCKET, key, options).expect("a location handle")
}

/// The canonical location of `key` in the fixture bucket.
pub fn location(key: &str) -> String {
    format!("s3://{BUCKET}/{key}")
}

/// Options that reach `store` as `provider`, consulting nothing outside the
/// test.
///
/// Each store is authorized the way that store is: S3 by its keys, Google by a
/// token the caller holds, Azure by the account key the emulators publish.
pub fn options_for(store: &FakeS3, provider: Provider) -> ObjectOptions {
    let options = ObjectOptions::default()
        .with_environment(false)
        .with_endpoint(store.endpoint())
        .with_region("us-east-1")
        .with_path_style(true);
    match provider {
        Provider::Aws => {
            options.with_credentials(Credentials::new("AKIAIOSFODNN7EXAMPLE", "wJalrXUtnFEMI"))
        }
        Provider::Google => options.with_google(
            GoogleOptions::default()
                .with_access_token("ya29.test")
                .with_project("trading"),
        ),
        Provider::Azure => options.with_azure(
            AzureOptions::default()
                .with_account(server::AZURE_ACCOUNT)
                .with_account_key(server::AZURE_KEY),
        ),
    }
}

/// The object `key` in the fixture container on `provider`.
pub fn file_on(store: &FakeS3, provider: Provider, key: &str) -> ObjectFile {
    file_on_with(provider, key, options_for(store, provider))
}

/// The object `key` on `provider`, under options a test tightened.
pub fn file_on_with(provider: Provider, key: &str, options: ObjectOptions) -> ObjectFile {
    object::file_with(&location_on(provider, key), options).expect("an object handle")
}

/// The prefix `key` in the fixture container on `provider`.
pub fn folder_on(store: &FakeS3, provider: Provider, key: &str) -> ObjectFolder {
    folder_on_with(provider, key, options_for(store, provider))
}

/// The prefix `key` on `provider`, under options a test tightened.
pub fn folder_on_with(provider: Provider, key: &str, options: ObjectOptions) -> ObjectFolder {
    object::folder_with(&location_on(provider, key), options).expect("a prefix handle")
}

/// The location `key` on `provider`, under options a test tightened.
pub fn path_on_with(provider: Provider, key: &str, options: ObjectOptions) -> ObjectPath {
    object::path_at_with(provider, BUCKET, key, options).expect("a location handle")
}

/// The canonical location of `key` in the fixture container, on `provider`.
pub fn location_on(provider: Provider, key: &str) -> String {
    format!("{}://{BUCKET}/{key}", provider.scheme().as_str())
}

/// A payload of `size` bytes that does not compress to nothing.
pub fn payload(size: usize) -> Vec<u8> {
    (0..size).map(|index| (index % 251) as u8).collect()
}

mod accounting {
    use yggdryl::IOBase;

    use crate::mod_::{file, folder, path, store};

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

    #[test]
    fn building_a_handle_costs_nothing() {
        let store = store();

        let file = file(&store, "lake/part.parquet");
        let folder = folder(&store, "lake/");
        let path = path(&store, "lake/part.parquet");

        // Not one request between the three of them, per the laziness contract.
        assert_eq!(store.request_count(), 0);
        // Nor does asking what a handle is called, or what it holds.
        assert_eq!(file.url().to_string(), "s3://trades/lake/part.parquet");
        assert_eq!(folder.prefix(), "lake/");
        assert_eq!(path.key(), "lake/part.parquet");
        assert_eq!(file.media_type().base(), &yggdryl::MimeType::PARQUET);
        assert!(!file.is_container());
        assert!(folder.is_container());
        assert_eq!(store.request_count(), 0);
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

        use std::sync::Arc;
        use yggdryl::StructType;

        use arrow_array::{Int64Array, RecordBatch, StringArray};

        use crate::mod_::BUCKET;
        use crate::server::FakeS3;
        use yggdryl::iceberg::{
            FormatVersion, IcebergOptions, PartitionSpec, Table, WriteStaging, assign_field_ids,
            read_manifest, write_manifest,
        };
        use yggdryl::object::ObjectFolder;
        use yggdryl::{DataType, Field, IOBase};

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
            let mut schema = StructType::from_fields([
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

        fn rows(ids: &[i64], venues: &[&str]) -> yggdryl::arrow::BatchReader {
            let batch = RecordBatch::try_new(
                schema().into_arrow_schema().expect("an Arrow schema"),
                vec![
                    Arc::new(Int64Array::from(ids.to_vec())),
                    Arc::new(StringArray::from(vec!["AAPL"; ids.len()])),
                    Arc::new(StringArray::from(venues.to_vec())),
                ],
            )
            .expect("the batch matches the schema");
            yggdryl::arrow::batch_reader(batch.schema(), [batch])
        }

        fn drain(reader: yggdryl::arrow::BatchReader) -> usize {
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
            let store = crate::mod_::store();
            let root: ObjectFolder = crate::mod_::folder(&store, "lake/trades/");
            let schema = schema();
            let spec = PartitionSpec::identity(1, &schema, &["venue"]).expect("venue is a column");

            // The claim, the document and the hint, the one listing that
            // detects a competing claim, and the claim's removal.
            let (mut table, create) = cost(&store, || {
                Table::create(root.clone(), FormatVersion::V2, schema.clone(), spec)
                    .expect("creates")
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
            let merge_by = yggdryl::Selector::from_columns(["id"]);
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
            let target: Field = StructType::from_fields([DataType::Int64.required_field("id")])
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
            let store = crate::mod_::store();
            let root: ObjectFolder = crate::mod_::folder(&store, "lake/trades/");
            let schema = schema();
            let spec = PartitionSpec::identity(1, &schema, &["venue"]).expect("venue is a column");
            let mut table =
                Table::create(root.clone(), FormatVersion::V2, schema, spec).expect("creates");
            let version = table.metadata_version();
            let stage = yggdryl::local::LocalFolder::temporary()
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
            let store = crate::mod_::store();
            let root: ObjectFolder = crate::mod_::folder(&store, "lake/trades/");
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
}

mod roles {
    use yggdryl::holder::buffered::BufferedOptions;
    use yggdryl::{IOBase, IOKind};

    use crate::mod_::{BUCKET, file, folder, options, path, payload, store};

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
        assert_eq!(handle.codec(), yggdryl::Codec::Gzip);

        let value = yggdryl::Scalar::from_struct([("symbol", yggdryl::Scalar::from("AAPL"))])
            .expect("a record");
        handle.write_scalar(&value).expect("a compressed write");
        assert_eq!(handle.read_scalar(None).expect("the value"), value);

        // What is stored is the compressed form, not the JSON.
        let stored = store
            .get(BUCKET, "lake/quotes.json.gz")
            .expect("the object");
        assert_eq!(&stored[..2], &[0x1f, 0x8b], "a gzip member");
    }

    #[test]
    fn records_round_trip_through_a_store_like_any_other_handle() {
        use std::sync::Arc;
        use yggdryl::IOMedia;

        let store = store();
        let field = yggdryl::StructType::from_fields([
            yggdryl::DataType::Int64.required_field("id"),
            yggdryl::DataType::utf8().required_field("symbol"),
        ])
        .map(yggdryl::DataType::from)
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
            .collect::<yggdryl::Result<Vec<_>>>()
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
        let root = yggdryl::holder::Holder::ObjectFolder(folder(&store, ""));

        assert!(root.is_container());
        assert_eq!(root.kind(), IOKind::Directory);
        let entries: Vec<_> = root
            .ls(true, false)
            .collect::<yggdryl::Result<Vec<_>>>()
            .expect("a listing");
        assert_eq!(entries.len(), 3, "two containers and the leaf");
        let leaf = entries.last().expect("the leaf");
        assert_eq!(leaf.read_all_bytes().expect("the object"), b"PAR1");
        assert!(matches!(leaf, yggdryl::holder::Holder::ObjectFile(_)));

        // Resolving down the tree stays in one type the whole way.
        let child = root
            .child_by_path("lake/year=2026/part.parquet")
            .expect("a child");
        assert_eq!(child.read_all_bytes().expect("the object"), b"PAR1");
    }

    #[test]
    fn a_bucket_root_is_a_container_that_creates_and_removes_the_bucket() {
        let store = crate::server::FakeS3::start();
        let mut root =
            yggdryl::object::folder_with("s3://fresh/", options(&store)).expect("a bucket handle");

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

        use yggdryl::IOCursor;
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
}
