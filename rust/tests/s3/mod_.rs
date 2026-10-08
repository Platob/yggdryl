//! `rust/src/s3/mod.rs`: the fixtures every object-store suite builds on.
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
//! `rust/src/s3/mod.rs` publishes - [`s3::file_with`],
//! [`s3::folder_with`], [`s3::located_with`] - so the fixtures are
//! themselves the pin on the crate's own door: a location a suite spells is
//! the location a handle reports.

#![allow(dead_code)]

use yggdryl::s3::{
    self, AzureOptions, Credentials, GoogleOptions, Provider, S3File, S3Folder, S3Options, S3Path,
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
pub fn options(store: &FakeS3) -> S3Options {
    S3Options::default()
        .with_environment(false)
        .with_endpoint(store.endpoint())
        .with_region("us-east-1")
        .with_path_style(true)
        .with_credentials(Credentials::new("AKIAIOSFODNN7EXAMPLE", "wJalrXUtnFEMI"))
}

/// The object `key` on `store`.
pub fn file(store: &FakeS3, key: &str) -> S3File {
    file_with(key, options(store))
}

/// The object `key`, under options a test tightened.
///
/// The options already name the endpoint, so the store is not passed again.
pub fn file_with(key: &str, options: S3Options) -> S3File {
    s3::file_with(&location(key), options).expect("an object handle")
}

/// The prefix `key` on `store`.
pub fn folder(store: &FakeS3, key: &str) -> S3Folder {
    folder_with(key, options(store))
}

/// The prefix `key`, under options a test tightened.
pub fn folder_with(key: &str, options: S3Options) -> S3Folder {
    s3::folder_with(&location(key), options).expect("a prefix handle")
}

/// The location `key` on `store`.
pub fn path(store: &FakeS3, key: &str) -> S3Path {
    path_with(key, options(store))
}

/// The location `key`, under options a test tightened.
pub fn path_with(key: &str, options: S3Options) -> S3Path {
    s3::path_at_with(Provider::Aws, BUCKET, key, options).expect("a location handle")
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
pub fn options_for(store: &FakeS3, provider: Provider) -> S3Options {
    let options = S3Options::default()
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
pub fn file_on(store: &FakeS3, provider: Provider, key: &str) -> S3File {
    file_on_with(provider, key, options_for(store, provider))
}

/// The object `key` on `provider`, under options a test tightened.
pub fn file_on_with(provider: Provider, key: &str, options: S3Options) -> S3File {
    s3::file_with(&location_on(provider, key), options).expect("an object handle")
}

/// The prefix `key` in the fixture container on `provider`.
pub fn folder_on(store: &FakeS3, provider: Provider, key: &str) -> S3Folder {
    folder_on_with(provider, key, options_for(store, provider))
}

/// The prefix `key` on `provider`, under options a test tightened.
pub fn folder_on_with(provider: Provider, key: &str, options: S3Options) -> S3Folder {
    s3::folder_with(&location_on(provider, key), options).expect("a prefix handle")
}

/// The location `key` on `provider`, under options a test tightened.
pub fn path_on_with(provider: Provider, key: &str, options: S3Options) -> S3Path {
    s3::path_at_with(provider, BUCKET, key, options).expect("a location handle")
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

    /// A text read of the objects under a glob: one listing of the prefix,
    /// then one `GET` per object - the listed leaf streamed through the
    /// resuming reader it owns, nothing reopened, no probe listing and no
    /// read past the end.
    #[test]
    fn a_text_read_of_a_glob_is_one_listing_and_one_get_per_object() {
        use yggdryl::IOMedia;
        use yggdryl::media::RecordOptions;
        use yggdryl::text::TextOptions;

        let store = store();
        store.put(crate::mod_::BUCKET, "logs/a.log", b"alpha\nbeta\n");
        store.put(crate::mod_::BUCKET, "logs/b.log", b"gamma\n");
        let logs = path(&store, "logs/*.log");
        let options = RecordOptions::from(TextOptions::new());
        store.clear_requests();
        let rows: usize = yggdryl::StreamChunkedSerie::from_serie(
            logs.read_serie(Some(&options)).expect("the lines"),
        )
        .expect("native record stream")
        .into_chunks()
        .map(|record| record.expect("a record").len())
        .sum();
        assert_eq!(rows, 3);
        let shapes: Vec<String> = store
            .requests()
            .iter()
            .map(|request| {
                let listing = request
                    .query
                    .iter()
                    .any(|(name, value)| name == "list-type" && value == "2");
                format!(
                    "{} {}",
                    if listing {
                        "LIST"
                    } else {
                        request.method.as_str()
                    },
                    request.key.clone().unwrap_or_default()
                )
            })
            .collect();
        assert_eq!(shapes, ["LIST ", "GET logs/a.log", "GET logs/b.log"]);
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
    /// The table never lists the store and never asks a file its role or its
    /// size: every file it touches is one the metadata names, so the counts
    /// below are the metadata chain - the hint, the document, the manifest list,
    /// the manifests - plus the data files a scan opens or a commit uploads,
    /// the one conditional `PUT` that claims a commit's version and the hint
    /// it replaces, and nothing else.
    #[cfg(feature = "iceberg")]
    mod iceberg {

        use std::sync::Arc;
        use yggdryl::StructType;

        use arrow_array::{Int64Array, RecordBatch, StringArray};

        use crate::mod_::BUCKET;
        use crate::server::FakeS3;
        use yggdryl::iceberg::{
            FormatVersion, IcebergOptions, IcebergTable, PartitionSpec, SchemaUpdate, WriteStaging,
            assign_field_ids, read_manifest, write_manifest,
        };
        use yggdryl::s3::S3Folder;
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
        /// footer read back from the store after each upload. Before a commit
        /// claimed its version with one conditional `PUT`, the create and the
        /// three commits cost two requests more each - an attempt document's
        /// `PUT` and a listing to find a competing one, and a `DELETE` - and
        /// the open two fewer, since it trusted the hint. Before the hint was
        /// written whole in one `PUT`, each commit cost a `DELETE` more, the
        /// hint removed before it was created again; before a claim read its
        /// version's other spelling, and an open read the current document's
        /// other spelling, each cost one `GET` fewer. Before a handle carried
        /// the manifest list its own last commit wrote, the second append and
        /// the upsert each read that list back, one `GET` more each; before a
        /// projection read a renamed table's file once, a projected scan
        /// after a rename read every data file twice, four `GET`s more.
        #[test]
        fn what_a_table_costs_over_the_store() {
            let store = crate::mod_::store();
            let root: S3Folder = crate::mod_::folder(&store, "lake/trades/");
            let schema = schema();
            let spec = PartitionSpec::identity(1, &schema, &["venue"]).expect("venue is a column");

            // One listing of `metadata/`, which finds no document under any
            // name - so the create hides no table laid out under another -
            // then the document's `PUT` under `If-None-Match: *`, the claim of
            // version 1, one `GET` of its other spelling, `v1.gz`, which finds
            // no claim beside it, and the hint's `PUT`, written whole.
            // v2: the sequence pins what a keyed merge costs, which v3 refuses.
            let (mut table, create) = cost(&store, || {
                IcebergTable::create(root.clone(), FormatVersion::V2, schema.clone(), spec)
                    .expect("creates")
            });
            pin(
                "create",
                &create,
                Tally {
                    total: 4,
                    put: 2,
                    get: 1,
                    list: 1,
                    ..Tally::default()
                },
            );
            assert!(
                table.write_staging().expect("resolves").folder().is_some(),
                "a remote root stages by default"
            );

            // One upload per file - the data file, the manifest, the manifest
            // list - then the hint read that re-checks the version, the
            // document's one conditional `PUT` that claims it, one `GET` of
            // the version's other spelling, and the hint's one `PUT`. Nothing
            // reads a footer or a size back, nothing is listed and nothing
            // removed.
            let ((), append_one) = cost(&store, || {
                table.commit_append(rows(&[1], &["XNAS"])).expect("appends")
            });
            pin(
                "append one partition",
                &append_one,
                Tally {
                    total: 7,
                    put: 5,
                    get: 2,
                    ..Tally::default()
                },
            );

            // Three data files; the manifests of the snapshot before are
            // carried from the list this handle's last commit wrote, read back
            // from nothing.
            let ((), append_three) = cost(&store, || {
                table
                    .commit_append(rows(&[2, 3, 4], &["XNAS", "XNYS", "XLON"]))
                    .expect("appends")
            });
            pin(
                "append three partitions",
                &append_three,
                Tally {
                    total: 9,
                    put: 7,
                    get: 2,
                    ..Tally::default()
                },
            );

            // The plan reads both manifests of the list this handle wrote, the
            // join reads the one file the key bounds keep, and the commit
            // writes one data file, its manifest, the carried manifest, the
            // list and the document.
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
                    total: 11,
                    put: 6,
                    get: 5,
                    ..Tally::default()
                },
            );

            // The hint, the document it names in both its spellings - the
            // gzip one answering `404`, read so that a version claimed under
            // two codecs is refused rather than chosen between - and the next
            // version's two spellings, which answer `404`: a hint is where the
            // search for the newest document starts. No listing of the
            // directory.
            let (opened, open) = cost(&store, || IcebergTable::open(root.clone()).expect("opens"));
            pin(
                "open",
                &open,
                Tally {
                    total: 5,
                    get: 5,
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

            // A renamed column costs a projection nothing more: the names a
            // file stores come off the footer read in the one read that takes
            // a file this short whole, and the record read decodes from those
            // bytes and that footer.
            table
                .commit_metadata_changes(|metadata| {
                    let mut update = SchemaUpdate::from_metadata(metadata)?;
                    update.rename_column("symbol", "ticker");
                    let evolved = update.into_field()?;
                    let schema_id = metadata.add_schema(evolved)?;
                    metadata.set_current_schema(schema_id)
                })
                .expect("renames a column");
            let renamed = IcebergTable::open(root.clone()).expect("opens");
            let (read, projected) = cost(&store, || {
                drain(renamed.scan(Some(&target)).expect("a projected scan"))
            });
            assert_eq!(read, 5);
            pin(
                "projected scan after a column rename",
                &projected,
                Tally {
                    total: 7,
                    get: 7,
                    ..Tally::default()
                },
            );
            assert_eq!(
                root.stats().lists,
                1,
                "nothing in the table's life lists but its create, once"
            );
        }

        /// Every id a scan reads, sorted.
        fn ids(reader: yggdryl::arrow::BatchReader) -> Vec<i64> {
            let mut ids: Vec<i64> = reader
                .flat_map(|batch| {
                    let batch = batch.expect("a batch");
                    batch
                        .column_by_name("id")
                        .expect("the id column")
                        .as_any()
                        .downcast_ref::<Int64Array>()
                        .expect("ids")
                        .values()
                        .to_vec()
                })
                .collect();
            ids.sort_unstable();
            ids
        }

        /// Eight writers, each holding version 1 of one table on the store,
        /// append twice each with nothing between them: the store's
        /// conditional `PUT` of each version's document is the only thing
        /// deciding who claims it, so every commit lands exactly once and the
        /// versions count the commits. Then one uncontended append costs what
        /// a commit costs: the race leaves nothing behind it to pay for.
        #[test]
        fn racing_appenders_over_the_store_each_land_every_commit_once() {
            const WRITERS: i64 = 8;
            const COMMITS: i64 = 2;
            let store = crate::mod_::store();
            let root: S3Folder = crate::mod_::folder(&store, "lake/racing/");
            IcebergTable::create(
                root.clone(),
                FormatVersion::V3,
                schema(),
                PartitionSpec::unpartitioned(),
            )
            .expect("creates");

            // A thousand attempts 1 to 16 ms apart inside ten minutes: the
            // budget is never what ends a commit, since of sixteen commits
            // one writer is beaten at most once per commit another lands.
            let racing = IcebergOptions::new()
                .with_commit_retries(1_000)
                .with_commit_min_backoff_ms(1)
                .with_commit_max_backoff_ms(16)
                .with_commit_total_timeout_ms(600_000);
            let start = Arc::new(std::sync::Barrier::new(WRITERS as usize));
            let handles: Vec<_> = (0..WRITERS)
                .map(|writer| {
                    let root = root.clone();
                    let start = Arc::clone(&start);
                    let racing = racing.clone();
                    std::thread::spawn(move || {
                        let mut table = IcebergTable::open(root).expect("opens");
                        table.set_options(racing);
                        start.wait();
                        for commit in 0..COMMITS {
                            table
                                .commit_append(rows(&[writer * 10 + commit], &["XNAS"]))
                                .unwrap_or_else(|error| {
                                    panic!("writer {writer}, commit {commit}: {error}")
                                });
                        }
                    })
                })
                .collect();
            for handle in handles {
                handle.join().expect("a writer");
            }

            let mut table = IcebergTable::open(root.clone()).expect("opens");
            let expected: Vec<i64> = (0..WRITERS)
                .flat_map(|writer| (0..COMMITS).map(move |commit| writer * 10 + commit))
                .collect();
            assert_eq!(ids(table.scan(None).expect("a scan")), expected);
            let version = table.metadata_version().unwrap();
            assert_eq!(version, 1 + (WRITERS * COMMITS) as u32);

            // The race may leave the hint naming an older version, which the
            // first commit after it reads past; the hint that commit writes
            // names the version the next one holds.
            table.set_options(racing);
            table
                .commit_append(rows(&[900], &["XNAS"]))
                .expect("appends");
            let ((), append) = cost(&store, || {
                table
                    .commit_append(rows(&[901], &["XNAS"]))
                    .expect("appends")
            });
            // One `PUT` each for the data file, the manifest and the manifest
            // list, the manifests of the snapshot before carried from the list
            // this handle's last commit wrote; one `GET` of the hint, which
            // names the version this handle holds; one `PUT` of the document
            // under `If-None-Match: *`, the claim of the version and the one
            // request that decides it under its spelling, and one `GET` of
            // the other spelling, which finds no claim beside it; and the
            // hint's one `PUT`, written whole.
            pin(
                "one uncontended append after the race",
                &append,
                Tally {
                    total: 7,
                    put: 5,
                    get: 2,
                    ..Tally::default()
                },
            );
            let claim = format!("lake/racing/metadata/v{}.metadata.json", version + 2);
            let claimed: Vec<_> = store
                .requests()
                .into_iter()
                .filter(|request| request.key.as_deref() == Some(claim.as_str()))
                .map(|request| {
                    let condition = request
                        .headers
                        .iter()
                        .find(|(name, _)| name == "if-none-match")
                        .map(|(_, value)| value.clone());
                    (request.method, condition, request.status)
                })
                .collect();
            assert_eq!(
                claimed,
                [("PUT".to_owned(), Some("*".to_owned()), 200)],
                "the claim is one conditional PUT"
            );
            assert_eq!(table.metadata_version().unwrap(), version + 2);
        }

        /// A refused upload fails the commit before anything else goes out: no
        /// manifest, no manifest list, no document, no orphan under `data/`,
        /// and nothing left in the staging folder.
        #[test]
        fn a_failed_upload_publishes_nothing_and_leaves_no_staged_file() {
            let store = crate::mod_::store();
            let root: S3Folder = crate::mod_::folder(&store, "lake/trades/");
            let schema = schema();
            let spec = PartitionSpec::identity(1, &schema, &["venue"]).expect("venue is a column");
            let mut table = IcebergTable::create(root.clone(), FormatVersion::V3, schema, spec)
                .expect("creates");
            let version = table.metadata_version().unwrap();
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
            assert_eq!(table.metadata_version().unwrap(), version);
            assert!(table.current_snapshot().unwrap().is_none());
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
            assert_eq!(table.metadata_version().unwrap(), version);
            assert!(table.current_snapshot().unwrap().is_none());
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
            assert_eq!(table.metadata_version().unwrap(), version + 1);
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
        /// length - in the read of its end, which states it - and its rows are
        /// read rather than taken for an empty file's none.
        #[test]
        fn a_manifest_recording_no_length_is_not_believed() {
            let store = crate::mod_::store();
            let root: S3Folder = crate::mod_::folder(&store, "lake/trades/");
            let schema = schema();
            let spec = PartitionSpec::identity(1, &schema, &["venue"]).expect("venue is a column");
            let mut table = IcebergTable::create(
                root.clone(),
                FormatVersion::V3,
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
            write_manifest(&mut manifest, FormatVersion::V3, &schema, &spec, &entries)
                .expect("the manifest rewrites");

            let opened = IcebergTable::open(root.clone()).expect("opens");
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
            // The list, the manifest, and the file's end, which states its
            // length: the `HEAD` the recorded length would have saved is gone.
            pin(
                "scan of a file recorded as empty",
                &scan,
                Tally {
                    total: 3,
                    get: 3,
                    ..Tally::default()
                },
            );
        }

        /// A commit that stages nothing costs what a staged one costs: every
        /// file goes straight to the store in one `PUT`, and nothing is asked
        /// back - a data file's statistics and length are what its encoder
        /// closed it with, a manifest's and a list's length what they were
        /// laid out as - so no `HEAD` sizes or probes a file. Before, each
        /// data file cost a `HEAD` probing its fresh name, a `HEAD` and a tail
        /// and a footer `GET` reading its statistics back and a `HEAD` for its
        /// size, and the manifest and the list a `HEAD` each.
        #[test]
        fn an_unstaged_commit_costs_what_a_staged_one_costs() {
            let store = crate::mod_::store();
            let root: S3Folder = crate::mod_::folder(&store, "lake/unstaged/");
            let schema = schema();
            let spec = PartitionSpec::identity(1, &schema, &["venue"]).expect("venue is a column");
            let mut table = IcebergTable::create(root.clone(), FormatVersion::V3, schema, spec)
                .expect("creates");
            table.set_options(
                IcebergOptions::new()
                    .try_with_write_staging(WriteStaging::Off)
                    .expect("off is accepted"),
            );

            // The data file, the manifest and the list, then the chain: the
            // hint read that re-checks the version, the document's claim, its
            // other spelling's `GET`, and the hint.
            let ((), append) = cost(&store, || {
                table.commit_append(rows(&[1], &["XNAS"])).expect("appends")
            });
            pin(
                "an unstaged append of one partition",
                &append,
                Tally {
                    total: 7,
                    put: 5,
                    get: 2,
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
        let root = yggdryl::holder::Holder::S3Folder(folder(&store, ""));

        assert!(root.is_container());
        assert_eq!(root.kind(), IOKind::Directory);
        let entries: Vec<_> = root
            .ls(true, false)
            .collect::<yggdryl::Result<Vec<_>>>()
            .expect("a listing");
        assert_eq!(entries.len(), 3, "two containers and the leaf");
        let leaf = entries.last().expect("the leaf");
        assert_eq!(leaf.read_all_bytes().expect("the object"), b"PAR1");
        assert!(matches!(leaf, yggdryl::holder::Holder::S3File(_)));

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
            yggdryl::s3::folder_with("s3://fresh/", options(&store)).expect("a bucket handle");

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

        // A folder of tabular leaves is tabular; it holds no bytes of its own,
        // and its whole read is the stream of the objects beneath it.
        assert!(lake.is_tabular());
        assert!(!lake.is_atomic());
        assert!(lake.is_io());
        assert_eq!(lake.size(), 0);
        assert_eq!(lake.read_all_bytes().expect("its leaves"), b"PAR1");
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
