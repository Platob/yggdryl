//! `rust/src/s3tables/catalog.rs`: a table bucket as a warehouse catalog.
//!
//! The control plane is the fake every suite here runs on; each table's
//! warehouse location is a bucket of the fake object store the `s3` suites
//! run on, so both ends of a commit - the files and the pointer - are
//! requests a test reads back. What the store is never asked - a listing, a
//! delete, a version hint - is pinned from its log.
//!
//! The second half is the bucket by its location alone: what
//! `s3tables://<bucket>[/<namespace>[/<table>]]`, a bucket's ARN and a
//! table's ARN each name, through every door that takes one, and what each
//! costs in requests.

use std::sync::Arc;

use arrow_array::{Array, FixedSizeBinaryArray, RecordBatch, TimestampNanosecondArray};

use crate::fake::{ACCESS_KEY, REGION, S3TablesFake, SECRET_KEY};
use crate::identity::Identity;
use crate::mod_::{LAKE_LABEL, client, lake, scratch};
use crate::server::FakeS3;
use yggdryl::aws::Session;
use yggdryl::holder::Holder;
use yggdryl::iceberg::FormatVersion;
use yggdryl::iceberg::{IcebergTable, PartitionSpec};
use yggdryl::s3tables::{S3Tables, S3TablesCatalog};
use yggdryl::{
    Arn, ArrowCastOptions, Catalog, CatalogValue, DataType, Field, IOBase, IOKind, IOMedia,
    NamespaceValue, ObjectValue, Properties, Serie, StructType, Table, TableValue, TimeUnit,
    Timezone, Url,
};

/// Fifteen minutes in nanoseconds: one partition's span.
const QUARTER: i64 = 15 * 60 * 1_000_000_000;

/// The fake object store every table's warehouse is a bucket of, made as
/// the control plane names one.
fn store() -> FakeS3 {
    let store = FakeS3::start();
    store.create_buckets_on_write(true);
    store
}

/// What reaches the fake store, stated on the catalog the way a caller
/// states it: the store's own names, read by the `s3` backend.
fn store_properties(store: &FakeS3) -> Properties {
    Properties::new()
        .with_property("s3.endpoint", store.endpoint())
        .with_property("path_style", "true")
}

/// The catalog of the fake's table bucket `lake`, its tables' storage on
/// `store`.
fn catalog(fake: &S3TablesFake, store: &FakeS3) -> Catalog {
    Catalog::from(
        S3TablesCatalog::new("lake", client(fake), lake(fake))
            .expect("a table bucket's ARN")
            .with_properties(store_properties(store)),
    )
}

/// The rows as a caller holds them: an instant to the nanosecond, a UUID
/// and a venue.
fn row() -> Field {
    DataType::from(
        StructType::from_fields([
            DataType::DateTime64 {
                unit: TimeUnit::Nanosecond,
                timezone: Timezone::UTC,
            }
            .required_field("ts"),
            DataType::uuid().required_field("id"),
            DataType::utf8().nullable_field("venue"),
        ])
        .expect("three columns"),
    )
    .required_field("row")
}

/// The table's schema: the rows, partitioned by the quarter-hour each
/// instant falls in, and sorted by it and the instant.
fn declared() -> Field {
    let mut schema = row()
        .with_partition_by(["time_bucket('15 minutes', ts) as part"
            .parse()
            .expect("a projection")])
        .expect("a partition");
    schema
        .as_sort_mut()
        .set_by_texts(["part", "ts"])
        .expect("an order");
    schema
}

/// One record of rows, `(instant, id)` each, venue `XNAS`.
fn rows(rows: &[(i64, u8)]) -> Serie {
    let schema = row().into_arrow_schema().expect("an Arrow schema");
    let batch = RecordBatch::try_new(
        Arc::clone(&schema),
        vec![
            Arc::new(
                TimestampNanosecondArray::from(rows.iter().map(|row| row.0).collect::<Vec<_>>())
                    .with_data_type(schema.field(0).data_type().clone()),
            ),
            Arc::new(
                FixedSizeBinaryArray::try_from_iter(rows.iter().map(|row| [row.1; 16]))
                    .expect("sixteen bytes each"),
            ),
            Arc::new(arrow_array::StringArray::from(vec!["XNAS"; rows.len()])),
        ],
    )
    .expect("a batch");
    Serie::from_arrow_batch(Some(&row()), &batch, ArrowCastOptions::default()).expect("a record")
}

/// `(part, ts, first id byte)` of every row a record read yields, in the
/// order it yields them.
fn read(table: &Table) -> Vec<(i64, i64, u8)> {
    let mut read = Vec::new();
    for record in yggdryl::StreamChunkedSerie::from_serie(table.read_serie(None).expect("a read"))
        .expect("native record stream")
        .into_chunks()
    {
        let batch = record
            .expect("a record")
            .into_arrow_batch()
            .expect("a batch");
        let stamped = |name: &str| {
            batch
                .column_by_name(name)
                .expect("the column")
                .as_any()
                .downcast_ref::<TimestampNanosecondArray>()
                .expect("instants")
                .clone()
        };
        let (parts, stamps) = (stamped("part"), stamped("ts"));
        let ids = batch.column_by_name("id").expect("the column");
        let ids = ids
            .as_any()
            .downcast_ref::<FixedSizeBinaryArray>()
            .expect("UUIDs");
        for index in 0..batch.num_rows() {
            read.push((parts.value(index), stamps.value(index), ids.value(index)[0]));
        }
    }
    read
}

/// Every request of the store that lists a bucket or removes from one.
fn forbidden(store: &FakeS3) -> Vec<String> {
    store
        .requests()
        .into_iter()
        .filter(|request| {
            request.method == "DELETE"
                || request
                    .query
                    .iter()
                    .any(|(name, _)| name == "delete" || name == "list-type")
                || (request.method == "GET" && request.key.as_deref().is_none_or(str::is_empty))
        })
        .map(|request| format!("{} {}", request.method, request.path))
        .collect()
}

/// Every key a write put in the store, in the order the writes went out.
fn written(store: &FakeS3) -> Vec<String> {
    store
        .requests()
        .into_iter()
        .filter(|request| request.method == "PUT" || request.method == "POST")
        .filter_map(|request| request.key)
        .collect()
}

#[test]
fn a_table_bucket_is_a_catalog_whose_tables_commit_through_the_control_plane() {
    let fake = S3TablesFake::start();
    let store = store();
    let catalog = catalog(&fake, &store);
    assert_eq!(catalog.namespace_levels(), Some(1));

    // A namespace is one request, and nothing reaches the store.
    fake.clear_requests();
    let desk = catalog
        .create_namespace("desk", &Properties::new())
        .expect("a namespace");
    assert_eq!(desk.to_string(), "lake.desk");
    assert_eq!(fake.request_count(), 1);

    // A table is registered with no document, and its first is the crate's:
    // version 0 under the warehouse the service chose, named current under
    // the token the service answered. The nanosecond instant makes it v3.
    fake.clear_requests();
    let mut table = desk
        .create_table("quotes", &declared(), &Properties::new())
        .expect("a table");
    assert_eq!(table.to_string(), "lake.desk.quotes");
    let lines = fake.lines();
    assert_eq!(lines.len(), 3, "{lines:?}");
    assert!(lines[0].starts_with("PUT /tables/"), "{lines:?}");
    assert!(
        lines[1].ends_with("/desk/quotes/metadata-location"),
        "{lines:?}"
    );
    assert!(lines[2].starts_with("PUT ") && lines[2].ends_with("/metadata-location"));
    let state = fake.table("lake", "desk", "quotes").expect("the table");
    assert!(state.schema.is_none(), "the service was sent no schema");
    let named = state.metadata_location.expect("a document named");
    assert!(
        named.starts_with(&format!("{}/metadata/00000-", state.warehouse_location))
            && named.ends_with(".metadata.json"),
        "{named}"
    );
    let bucket = state
        .warehouse_location
        .strip_prefix("s3://")
        .expect("an s3 location")
        .to_owned();
    let Table::Iceberg(iceberg) = &table else {
        panic!("expected an Iceberg table, got {table:?}");
    };
    assert_eq!(
        iceberg.metadata().expect("its document").format_version(),
        FormatVersion::V3
    );
    assert_eq!(iceberg.metadata_version().expect("its version"), 0);
    assert_eq!(iceberg.metadata_location().expect("its location"), named);

    // Two appends and an overwrite of the second quarter: each commit is the
    // files it writes and one publication, and the overwrite replaces the
    // quarter its rows fall in and no other.
    table
        .append_serie(rows(&[(QUARTER + 7, 3), (2, 1)]), None)
        .expect("an append");
    table
        .append_serie(rows(&[(QUARTER + 1, 4), (5, 2)]), None)
        .expect("an append");
    table
        .overwrite_serie(rows(&[(QUARTER + 9, 9)]), None)
        .expect("an overwrite");
    let committed = fake.table("lake", "desk", "quotes").expect("the table");
    assert!(
        committed
            .metadata_location
            .as_deref()
            .is_some_and(|location| location.contains("/metadata/00003-")),
        "{committed:?}"
    );

    // Partition after partition, each in the table's order.
    assert_eq!(
        read(&table),
        [(0, 2, 1), (0, 5, 2), (QUARTER, QUARTER + 9, 9)]
    );

    // Reopened through the catalog: one request - where the table's
    // document is, the namespace descended by description - and the same
    // rows read through the document that request named, asking nothing
    // more.
    fake.clear_requests();
    let reopened = catalog.table("desk.quotes").expect("the table");
    assert_eq!(fake.lines(), [metadata_location_line("quotes")]);
    assert_eq!(read(&reopened), read(&table));
    assert_eq!(fake.request_count(), 1, "{:?}", fake.lines());
    assert_eq!(
        reopened
            .field()
            .expect("its schema")
            .get_metadata("PARTITION:by"),
        Some(r#"["part"]"#)
    );

    // The catalog keeps the table: its own listing is refused rather than
    // reaching the store, and the service still names it.
    let kept = reopened;
    let listed: Vec<_> = kept.ls(true, false).collect();
    assert!(
        listed.len() == 1 && listed[0].is_err(),
        "one refusal, got {} entries",
        listed.len()
    );
    assert_eq!(read(&kept), read(&table));

    // Nothing ever listed the store or removed from it, and no hint was
    // written: four documents, and the files each commit named.
    assert_eq!(forbidden(&store), Vec::<String>::new());
    let keys = written(&store);
    assert!(
        keys.iter().all(|key| !key.ends_with("version-hint.text")),
        "{keys:?}"
    );
    assert_eq!(
        keys.iter()
            .filter(|key| key.ends_with(".metadata.json"))
            .count(),
        4,
        "{keys:?}"
    );
    assert!(
        store
            .keys(&bucket)
            .iter()
            .any(|key| key.starts_with("data/")),
        "{:?}",
        store.keys(&bucket)
    );
}

#[test]
fn a_commit_under_a_token_the_table_moved_past_is_a_conflict_or_a_rebase() {
    let fake = S3TablesFake::start();
    let store = store();
    let catalog = catalog(&fake, &store);
    let desk = catalog
        .create_namespace("desk", &Properties::new())
        .expect("a namespace");
    let mut first = desk
        .create_table("quotes", &declared(), &Properties::new())
        .expect("a table");
    let mut second = catalog.table("desk.quotes").expect("the table");
    assert_eq!(read(&second), []);

    // The first commits; the second still holds version 0 and its token.
    first
        .append_serie(rows(&[(1, 1)]), None)
        .expect("an append");

    // An overwrite planned against version 0 is refused rather than
    // replacing what the first wrote...
    let error = second
        .overwrite_serie(rows(&[(2, 2)]), None)
        .expect_err("a stale token");
    assert!(error.is_conflict(), "{error}");
    assert!(error.to_string().contains("last saw version 1"), "{error}");

    // ...while an append reads where the table stands and applies again.
    second
        .append_serie(rows(&[(3, 3)]), None)
        .expect("a rebased append");
    assert_eq!(read(&second), [(0, 1, 1), (0, 3, 3)]);
    let state = fake.table("lake", "desk", "quotes").expect("the table");
    assert!(
        state
            .metadata_location
            .as_deref()
            .is_some_and(|location| location.contains("/metadata/00002-")),
        "{state:?}"
    );
    assert_eq!(forbidden(&store), Vec::<String>::new());
}

/// Eight handles on one table, three appends each with nothing between
/// them: the control plane's version token is the only thing deciding which
/// publication lands, so every row lands once, every publication either
/// lands or is a conflict, each conflict costs one reading of where the
/// pointer stands and one read of the document it names, and the store is
/// never listed nor deleted from.
#[test]
fn racing_handles_publish_every_append_once_through_the_control_plane() {
    const HANDLES: u8 = 8;
    const APPENDS: u8 = 3;
    let fake = S3TablesFake::start();
    let store = store();
    let catalog = catalog(&fake, &store);
    catalog
        .create_namespace("desk", &Properties::new())
        .expect("a namespace");
    catalog
        .tables()
        .create("desk.quotes", &declared(), &Properties::new())
        .expect("a table");

    // A thousand attempts 1 to 16 ms apart inside ten minutes: the budget is
    // never what ends a commit, since of twenty-four commits one handle is
    // beaten at most once per commit another lands. Each handle reads its
    // document before the race, so what the race reads is the race's.
    let racing = yggdryl::iceberg::IcebergOptions::new()
        .with_commit_retries(1_000)
        .with_commit_min_backoff_ms(1)
        .with_commit_max_backoff_ms(16)
        .with_commit_total_timeout_ms(600_000);
    let tables: Vec<Table> = (0..HANDLES)
        .map(|_| {
            let mut table = catalog.table("desk.quotes").expect("the table");
            let Table::Iceberg(iceberg) = &mut table else {
                panic!("expected an Iceberg table, got {table:?}");
            };
            iceberg.set_options(racing.clone());
            assert_eq!(iceberg.metadata_version().expect("its version"), 0);
            table
        })
        .collect();
    fake.clear_requests();
    store.clear_requests();

    let start = Arc::new(std::sync::Barrier::new(usize::from(HANDLES)));
    let threads: Vec<_> = tables
        .into_iter()
        .enumerate()
        .map(|(handle, mut table)| {
            let start = Arc::clone(&start);
            std::thread::spawn(move || {
                start.wait();
                for append in 0..APPENDS {
                    let id = u8::try_from(handle).expect("eight handles") * APPENDS + append + 1;
                    table
                        .append_serie(rows(&[(i64::from(id), id)]), None)
                        .unwrap_or_else(|error| {
                            panic!("handle {handle}, append {append}: {error}")
                        });
                }
            })
        })
        .collect();
    for thread in threads {
        thread.join().expect("a handle");
    }

    // Every publication went to the control plane and either landed or was
    // refused as moved; each refusal was followed by one reading of where
    // the pointer stands, then one read of the document it named.
    let publications: Vec<u16> = fake
        .requests()
        .into_iter()
        .filter(|request| request.method == "PUT" && request.path.ends_with("/metadata-location"))
        .map(|request| request.status)
        .collect();
    let landed = publications.iter().filter(|status| **status == 200).count();
    let conflicts = publications.iter().filter(|status| **status == 409).count();
    assert_eq!(landed, usize::from(HANDLES * APPENDS), "{publications:?}");
    assert_eq!(landed + conflicts, publications.len(), "{publications:?}");
    println!("racing handles: {landed} publications landed, {conflicts} refused as moved");
    let readings = fake
        .requests()
        .into_iter()
        .filter(|request| request.method == "GET" && request.path.ends_with("/metadata-location"))
        .count();
    assert_eq!(
        readings, conflicts,
        "one reading of the pointer per conflict"
    );
    let documents_read = store
        .requests()
        .into_iter()
        .filter(|request| {
            request.method == "GET"
                && request
                    .key
                    .as_deref()
                    .is_some_and(|key| key.ends_with(".metadata.json"))
        })
        .count();
    assert_eq!(
        documents_read, conflicts,
        "one read of the winner's document per conflict"
    );
    assert_eq!(forbidden(&store), Vec::<String>::new());

    // Twenty-four publications on top of the first document, and every row
    // read once.
    let state = fake.table("lake", "desk", "quotes").expect("the table");
    assert!(
        state
            .metadata_location
            .as_deref()
            .is_some_and(|location| location.contains("/metadata/00024-")),
        "{state:?}"
    );
    let mut ids: Vec<u8> = read(&catalog.table("desk.quotes").expect("the table"))
        .into_iter()
        .map(|(_, _, id)| id)
        .collect();
    ids.sort_unstable();
    assert_eq!(ids, (1..=HANDLES * APPENDS).collect::<Vec<u8>>());
}

#[test]
fn a_create_states_its_format_version_or_takes_the_lowest_the_schema_needs() {
    let fake = S3TablesFake::start();
    let store = store();
    let catalog = catalog(&fake, &store);
    let desk = catalog
        .create_namespace("desk", &Properties::new())
        .expect("a namespace");
    let micros = DataType::from(
        StructType::from_fields([DataType::Int64.required_field("id")]).expect("a column"),
    )
    .required_field("row");
    let version = |table: &Table| match table {
        Table::Iceberg(table) => table.metadata().expect("its document").format_version(),
        other => panic!("expected an Iceberg table, got {other:?}"),
    };

    let plain = desk
        .create_table("plain", &micros, &Properties::new())
        .expect("a table");
    // v2: the create default for a schema that needs no v3 type.
    assert_eq!(version(&plain), FormatVersion::V2);
    let stated = desk
        .create_table(
            "stated",
            &micros,
            &Properties::new().with_property("format-version", "3"),
        )
        .expect("a table");
    assert_eq!(version(&stated), FormatVersion::V3);

    // A version no format has is refused before anything is sent.
    fake.clear_requests();
    let error = desk
        .create_table(
            "refused",
            &micros,
            &Properties::new().with_property("format-version", "7"),
        )
        .expect_err("no format version 7");
    assert!(
        error.to_string().contains("$.with.format-version"),
        "{error}"
    );
    assert_eq!(fake.request_count(), 0);
}

#[test]
fn listing_and_reading_a_level_is_one_request_per_page_and_one_per_table() {
    let fake = S3TablesFake::start();
    let store = store();
    let catalog = catalog(&fake, &store);
    let desk = catalog
        .create_namespace("desk", &Properties::new())
        .expect("a namespace");
    for name in ["orders", "quotes"] {
        desk.create_table(name, &declared(), &Properties::new())
            .expect("a table");
    }

    fake.clear_requests();
    let namespaces: Vec<String> = catalog
        .children()
        .map(|child| child.expect("a namespace").name().to_owned())
        .collect();
    assert_eq!(namespaces, ["desk"]);
    assert_eq!(fake.request_count(), 1);

    fake.clear_requests();
    let tables: Vec<String> = desk
        .children()
        .map(|child| child.expect("a table").name().to_owned())
        .collect();
    assert_eq!(tables, ["orders", "quotes"]);
    assert_eq!(fake.request_count(), 3, "{:?}", fake.lines());

    // What is not there is absent by its warehouse path.
    let error = catalog.table("desk.missing").expect_err("no such table");
    assert!(error.is_absent(), "{error}");
    assert!(error.to_string().contains("lake.desk.missing"), "{error}");
    let error = catalog.namespace("missing").expect_err("no such namespace");
    assert!(error.is_absent(), "{error}");
}

#[test]
fn a_table_bucket_location_is_its_catalog_under_the_properties_stated() {
    let fake = S3TablesFake::start();
    let arn = lake(&fake).to_string();
    let aws = scratch("from-url");
    let properties = Properties::new()
        .with_property("access_key_id", ACCESS_KEY)
        .with_property("secret_access_key", SECRET_KEY)
        .with_property("config_file", aws.join("config").display().to_string())
        .with_property(
            "shared_credentials_file",
            aws.join("credentials").display().to_string(),
        )
        .with_property("use_fips_endpoint", "false")
        .with_property("use_dualstack_endpoint", "false")
        .with_property("s3tables.region", REGION)
        .with_property("s3tables.endpoint", fake.endpoint());
    let location = Url::from_location(&arn).expect("a table bucket's location");
    assert_eq!(location.to_string(), "s3tables://lake");

    // Named by the location alone: no request until the ARN is needed, and
    // then one listing of the caller's own buckets, once.
    let catalog = Catalog::from_url(&location, &properties).expect("a catalog");
    assert_eq!(catalog.name(), "lake");
    assert_eq!(catalog.namespace_levels(), Some(1));
    assert_eq!(fake.request_count(), 0);
    let Catalog::S3Tables(bucket) = &catalog else {
        panic!("expected an S3 Tables catalog");
    };
    assert_eq!(bucket.bucket_arn().expect("the ARN").to_string(), arn);
    assert_eq!(bucket.bucket_arn().expect("the ARN").to_string(), arn);
    assert_eq!(fake.lines(), ["GET /buckets?maxBuckets=250"]);

    // Who signs is the session's: the catalog keeps none of the identity
    // properties it was stated, so it lists and prints no secret.
    use yggdryl::ObjectValue as _;
    let kept = catalog.properties().expect("the catalog's properties");
    assert!(
        kept.iter()
            .all(|(name, _)| !yggdryl::aws::Session::is_property(name)),
        "an identity property kept in {kept:?}"
    );
    let shown = format!("{catalog:?}");
    assert!(!shown.contains(SECRET_KEY), "a secret in {shown}");

    // Named by its ARN, the location keeps the region and the account the
    // ARN states: the catalog is the same bucket, and no request asks for
    // either - as text, as an `Arn`, or as a `Uri`.
    fake.clear_requests();
    let named = Catalog::from_url(
        yggdryl::Uri::from_str(&arn).expect("an identifier"),
        &properties,
    )
    .expect("a catalog");
    assert_eq!(named.name(), "lake");
    for named in [
        named,
        Catalog::from_url(yggdryl::Arn::from_str(&arn).expect("an ARN"), &properties)
            .expect("a catalog"),
    ] {
        let Catalog::S3Tables(bucket) = named else {
            panic!("expected an S3 Tables catalog");
        };
        assert_eq!(bucket.bucket_arn().expect("the ARN").to_string(), arn);
        assert_eq!(
            bucket.url().map(ToString::to_string).as_deref(),
            Some("s3tables://lake")
        );
    }
    assert_eq!(fake.request_count(), 0, "{:?}", fake.lines());

    // A `warehouse` property beside it must name that ARN.
    let error = Catalog::from_url(
        yggdryl::Uri::from_str(&arn).expect("an identifier"),
        &properties
            .clone()
            .with_property("warehouse", arn.replacen(REGION, "ap-south-2", 1).as_str()),
    )
    .expect_err("two ARNs of one bucket");
    assert!(error.to_string().contains("$.with.warehouse"), "{error}");
    assert!(error.to_string().contains("the location states"), "{error}");

    // An account, or the ARN itself, states it with no request at all.
    fake.clear_requests();
    for stated in [
        properties
            .clone()
            .with_property("account_id", crate::fake::ACCOUNT),
        properties.clone().with_property("warehouse", arn.as_str()),
        properties
            .clone()
            .with_property("s3tables.warehouse", arn.as_str()),
    ] {
        let Catalog::S3Tables(bucket) = Catalog::from_url(&location, &stated).expect("a catalog")
        else {
            panic!("expected an S3 Tables catalog");
        };
        assert_eq!(bucket.bucket_arn().expect("the ARN").to_string(), arn);
    }
    assert_eq!(fake.request_count(), 0);

    // An ARN naming another bucket, or a table below this one, is refused.
    let error = Catalog::from_url(
        &location,
        &properties.clone().with_property(
            "warehouse",
            "arn:aws:s3tables:us-east-1:123456789012:bucket/other",
        ),
    )
    .expect_err("another bucket");
    assert!(error.to_string().contains("$.with.warehouse"), "{error}");
    let error = Catalog::from_url(
        Url::from_str("s3tables://lake/t-a1").expect("a table's location"),
        &properties,
    )
    .expect_err("a table, not a bucket");
    assert!(error.to_string().contains("$.url"), "{error}");
    let _ = std::fs::remove_dir_all(&aws);
}

/// What a location door is handed for the two fakes: who signs, where the
/// control plane and the store are, and shared files of this test's own, so
/// nothing of the operator's is read.
fn stated(fake: &S3TablesFake, store: &FakeS3, aws: &std::path::Path) -> Properties {
    store_properties(store)
        .with_property("access_key_id", ACCESS_KEY)
        .with_property("secret_access_key", SECRET_KEY)
        .with_property("config_file", aws.join("config").display().to_string())
        .with_property(
            "shared_credentials_file",
            aws.join("credentials").display().to_string(),
        )
        .with_property("use_fips_endpoint", "false")
        .with_property("use_dualstack_endpoint", "false")
        .with_property("s3tables.region", REGION)
        .with_property("s3tables.endpoint", fake.endpoint())
}

/// `text` as the location it spells.
fn url(text: &str) -> Url {
    Url::from_str(text).expect("a location")
}

/// The line `GetTableMetadataLocation` of `desk.<name>` leaves in the log.
fn metadata_location_line(name: &str) -> String {
    format!("GET /tables/{LAKE_LABEL}/desk/{name}/metadata-location")
}

/// The line `GetNamespace` of `<name>` leaves in the log.
fn get_namespace_line(name: &str) -> String {
    format!("GET /namespaces/{LAKE_LABEL}/{name}")
}

/// Every `GET` of an object the store answered, by key.
fn fetched(store: &FakeS3) -> Vec<String> {
    store
        .requests()
        .into_iter()
        .filter(|request| request.method == "GET")
        .filter_map(|request| request.key)
        .collect()
}

#[test]
fn a_location_names_the_catalog_the_namespace_or_the_table() {
    let fake = S3TablesFake::start();
    let store = store();
    let arn = lake(&fake).to_string();
    let aws = scratch("locate");
    // The bucket's ARN stated beside the location: nothing asks for it.
    let properties = stated(&fake, &store, &aws).with_property("warehouse", arn.as_str());
    let desk = catalog(&fake, &store)
        .create_namespace("desk", &Properties::new())
        .expect("a namespace");
    desk.create_table("quotes", &declared(), &Properties::new())
        .expect("a table")
        .append_serie(rows(&[(2, 1)]), None)
        .expect("an append");

    // The bucket is its catalog and a segment below it a namespace: a
    // description each, and no request - a trailing slash names nothing.
    fake.clear_requests();
    let held = Holder::from_url(url("s3tables://lake"), &properties).expect("the catalog");
    let Holder::Catalog(bucket) = &held else {
        panic!("expected a catalog, got {held:?}");
    };
    assert_eq!(bucket.to_string(), "lake");
    assert_eq!(held.kind(), IOKind::Catalog);
    for spelled in ["s3tables://lake/desk", "s3tables://lake/desk/"] {
        let held = Holder::from_url(url(spelled), &properties).expect("the namespace");
        let Holder::Namespace(namespace) = &held else {
            panic!("expected a namespace, got {held:?}");
        };
        assert_eq!(namespace.to_string(), "lake.desk");
        assert_eq!(held.kind(), IOKind::Namespace);
    }
    assert_eq!(fake.request_count(), 0, "{:?}", fake.lines());

    // A table is one request - where its document is - and no `GetNamespace`.
    let held =
        Holder::from_url(url("s3tables://lake/desk/quotes"), &properties).expect("the table");
    let Holder::Table(table) = &held else {
        panic!("expected a table, got {held:?}");
    };
    assert_eq!(table.to_string(), "lake.desk.quotes");
    assert_eq!(fake.lines(), [metadata_location_line("quotes")]);

    // Its first read asks the pointer nothing - the request that located
    // the table primed it - and reads the one document that request named;
    // what was read is kept.
    fake.clear_requests();
    store.clear_requests();
    let schema = table.field().expect("its schema");
    assert_eq!(schema.get_metadata("PARTITION:by"), Some(r#"["part"]"#));
    assert_eq!(fake.request_count(), 0, "{:?}", fake.lines());
    let documents = fetched(&store);
    assert!(
        documents.len() == 1 && documents[0].ends_with(".metadata.json"),
        "{documents:?}"
    );
    // That one `GetObject` is the whole of what the store was asked: no
    // `HEAD`, no listing beside it.
    assert_eq!(store.request_count(), 1, "{:?}", store.requests());
    table.field().expect("its schema");
    assert_eq!(fake.request_count(), 0);
    assert_eq!(store.request_count(), 1, "{:?}", store.requests());
    assert_eq!(read(table), [(0, 2, 1)]);

    // The catalog is named as the `name` property says.
    let named = Holder::from_url(
        url("s3tables://lake/desk"),
        &properties.clone().with_property("name", "prod"),
    )
    .expect("the namespace");
    let Holder::Namespace(namespace) = &named else {
        panic!("expected a namespace, got {named:?}");
    };
    assert_eq!(namespace.to_string(), "prod.desk");

    // What is not there is absent by its warehouse path, at one request.
    fake.clear_requests();
    let error = Holder::from_url(url("s3tables://lake/desk/missing"), &properties)
        .expect_err("no such table");
    assert!(error.is_absent(), "{error}");
    assert!(error.to_string().contains("lake.desk.missing"), "{error}");
    assert_eq!(fake.lines(), [metadata_location_line("missing")]);

    // A location spells at most a namespace and a table below its bucket,
    // and each by a name the service has: refused with no request.
    fake.clear_requests();
    for refused in [
        "s3tables://lake/a/b/c",
        "s3tables://lake/desk/quotes/extra/",
        "s3tables://lake/Desk",
        "s3tables://lake/desk/no-hyphen",
    ] {
        let error = Holder::from_url(url(refused), &properties).expect_err("no such place");
        assert!(error.to_string().contains("$.url"), "{refused}: {error}");
        assert!(
            error
                .to_string()
                .contains("s3tables://<bucket>[/<namespace>[/<table>]]"),
            "{refused}: {error}"
        );
    }
    assert_eq!(fake.request_count(), 0, "{:?}", fake.lines());
    let _ = std::fs::remove_dir_all(&aws);
}

#[test]
fn a_bare_location_finds_its_bucket_once() {
    let fake = S3TablesFake::start();
    let store = store();
    lake(&fake);
    let aws = scratch("bare");
    let properties = stated(&fake, &store, &aws);
    let desk = catalog(&fake, &store)
        .create_namespace("desk", &Properties::new())
        .expect("a namespace");
    for name in ["orders", "quotes"] {
        desk.create_table(name, &declared(), &Properties::new())
            .expect("a table");
    }

    // A location alone states no account and no region: the one listing a
    // catalog named so pays finds the bucket's ARN, then the table's
    // request.
    fake.clear_requests();
    Holder::from_url(url("s3tables://lake/desk/quotes"), &properties).expect("the table");
    assert_eq!(
        fake.lines(),
        [
            "GET /buckets?maxBuckets=250".to_owned(),
            metadata_location_line("quotes")
        ]
    );

    // The namespace keeps what it found: every table below it is one
    // request, and the listing is never asked again.
    let held = Holder::from_url(url("s3tables://lake/desk"), &properties).expect("a namespace");
    let Holder::Namespace(namespace) = &held else {
        panic!("expected a namespace, got {held:?}");
    };
    fake.clear_requests();
    for name in ["orders", "quotes"] {
        namespace.tables().get(name).expect("the table");
    }
    assert_eq!(
        fake.lines(),
        [
            "GET /buckets?maxBuckets=250".to_owned(),
            metadata_location_line("orders"),
            metadata_location_line("quotes")
        ]
    );

    // An account stated beside the location saves the listing.
    fake.clear_requests();
    Holder::from_url(
        url("s3tables://lake/desk/quotes"),
        &properties
            .clone()
            .with_property("account_id", crate::fake::ACCOUNT),
    )
    .expect("the table");
    assert_eq!(fake.lines(), [metadata_location_line("quotes")]);
    let _ = std::fs::remove_dir_all(&aws);
}

#[test]
fn a_bare_location_lists_once_whichever_door_opens_or_creates_it() {
    let fake = S3TablesFake::start();
    let store = store();
    lake(&fake);
    let aws = scratch("bare-doors");
    let properties = stated(&fake, &store, &aws);
    catalog(&fake, &store)
        .create_namespace("desk", &Properties::new())
        .expect("a namespace");
    let listed = "GET /buckets?maxBuckets=250".to_owned();
    let created = format!("PUT /tables/{LAKE_LABEL}/desk");
    let published = |name: &str| format!("PUT /tables/{LAKE_LABEL}/desk/{name}/metadata-location");

    // A create by the location alone: the one listing that finds the
    // bucket's ARN, then the creation's own three, and the store's one
    // `PutObject`.
    fake.clear_requests();
    store.clear_requests();
    IcebergTable::create_from_url(
        url("s3tables://lake/desk/quotes"),
        &properties,
        None,
        plain(),
        None,
    )
    .expect("a table");
    assert_eq!(
        fake.lines(),
        [
            listed.clone(),
            created.clone(),
            metadata_location_line("quotes"),
            published("quotes"),
        ]
    );
    assert_eq!(written(&store).len(), 1, "{:?}", store.requests());
    assert_eq!(store.request_count(), 1, "{:?}", store.requests());

    // Opening or creating a table that is not there reads the location and
    // resolves the bucket once for both halves: one listing, the open's
    // refused request, then the creation's three - never a second listing -
    // and the store's one `PutObject`.
    fake.clear_requests();
    store.clear_requests();
    IcebergTable::open_or_create_from_url(
        url("s3tables://lake/desk/fills"),
        &properties,
        None,
        plain(),
        None,
    )
    .expect("a table");
    assert_eq!(
        fake.lines(),
        [
            listed.clone(),
            metadata_location_line("fills"),
            created.clone(),
            metadata_location_line("fills"),
            published("fills"),
        ]
    );
    assert_eq!(fake.requests()[1].status, 404);
    assert_eq!(written(&store).len(), 1, "{:?}", store.requests());
    assert_eq!(store.request_count(), 1, "{:?}", store.requests());

    // One that is there is the listing and the open.
    fake.clear_requests();
    IcebergTable::open_or_create_from_url(
        url("s3tables://lake/desk/fills"),
        &properties,
        None,
        plain(),
        None,
    )
    .expect("the table");
    assert_eq!(
        fake.lines(),
        [listed.clone(), metadata_location_line("fills")]
    );

    // A bucket the caller has none of is absent at the one listing that
    // says so: nothing is opened, and nothing is created under it.
    fake.clear_requests();
    let error = IcebergTable::open_or_create_from_url(
        url("s3tables://nowhere/desk/quotes"),
        &properties,
        None,
        plain(),
        None,
    )
    .expect_err("no such bucket");
    assert!(error.is_absent(), "{error}");
    assert!(error.to_string().contains("table bucket"), "{error}");
    assert_eq!(fake.lines(), [listed]);
    assert_eq!(forbidden(&store), Vec::<String>::new());
    let _ = std::fs::remove_dir_all(&aws);
}

/// A store's own names stated to a location door - PyIceberg's `s3.` pair
/// among them - open every table's storage and stand nowhere else: the
/// catalog, its namespace and its table list and print none of them, and
/// the store takes only that pair, which proves where it went.
#[test]
fn a_stores_own_credential_opens_the_storage_and_is_listed_and_printed_by_nothing() {
    // AWS's second published example pair, the store's alone.
    const STORE_KEY: &str = "AKIAI44QH8DHBEXAMPLE";
    const STORE_SECRET: &str = "je7MtGbClwBF/2Zp9Utk/h3yCo8nvbEXAMPLEKEY";
    let fake = S3TablesFake::start();
    let store = store();
    lake(&fake);
    let aws = scratch("store-pair");
    let properties = stated(&fake, &store, &aws)
        .with_property("s3.access-key-id", STORE_KEY)
        .with_property("s3.secret-access-key", STORE_SECRET)
        .with_property("owner", "ops");
    catalog(&fake, &store)
        .create_namespace("desk", &Properties::new())
        .expect("a namespace");
    store.require_access_key(Some(STORE_KEY));

    let mut table = IcebergTable::create_from_url(
        url("s3tables://lake/desk/quotes"),
        &properties,
        None,
        declared(),
        None,
    )
    .expect("a table");
    table
        .append_serie(rows(&[(2, 1)]), None)
        .expect("an append signed as the store's pair");

    let catalog = Catalog::from_url(url("s3tables://lake"), &properties).expect("the catalog");
    let Holder::Namespace(namespace) =
        Holder::from_url(url("s3tables://lake/desk"), properties.iter()).expect("the namespace")
    else {
        panic!("expected a namespace");
    };
    use yggdryl::ObjectValue as _;
    for (what, listed, printed) in [
        (
            "catalog",
            catalog.properties().expect("properties"),
            format!("{catalog:?}"),
        ),
        (
            "namespace",
            namespace.properties().expect("properties"),
            format!("{namespace:?}"),
        ),
        (
            "table",
            ObjectValue::properties(&table).expect("properties"),
            format!("{table:?}"),
        ),
    ] {
        assert_eq!(listed.get("owner"), Some("ops"), "{what}");
        for name in [
            "s3.access-key-id",
            "s3.secret-access-key",
            "s3.endpoint",
            "path_style",
        ] {
            assert_eq!(listed.get(name), None, "{what}: {name} in {listed:?}");
        }
        for secret in [STORE_SECRET, STORE_KEY, SECRET_KEY] {
            assert!(!printed.contains(secret), "{what}: {printed}");
        }
    }
    assert_eq!(read(&Table::from(table)), [(0, 2, 1)]);
    assert_eq!(forbidden(&store), Vec::<String>::new());
    let _ = std::fs::remove_dir_all(&aws);
}

/// A table bucket's ARN is `bucket/<name>` and nothing below it: a trailing
/// slash, an empty identifier or a name where an identifier goes is refused
/// where the ARN is read, at no request, by every door that takes one.
#[test]
fn a_bucket_arn_with_anything_below_its_name_is_refused_where_it_is_read() {
    let fake = S3TablesFake::start();
    let store = store();
    let aws = scratch("arn-shape");
    let properties = stated(&fake, &store, &aws);
    let lake = lake(&fake).to_string();
    for below in ["/", "/table/", "/table", "/quotes"] {
        let arn = Arn::from_str(&format!("{lake}{below}")).expect("an ARN that parses");
        let refusals = [
            Holder::from_url(&arn, properties.iter())
                .expect_err("no table bucket")
                .to_string(),
            IcebergTable::from_url(&arn, &properties)
                .expect_err("no table bucket")
                .to_string(),
            S3TablesCatalog::new("lake", client(&fake), arn.clone())
                .expect_err("no table bucket")
                .to_string(),
        ];
        for refusal in refusals {
            assert!(refusal.contains("table bucket"), "{below}: {refusal}");
        }
    }
    assert_eq!(fake.request_count(), 0, "{:?}", fake.lines());
    let _ = std::fs::remove_dir_all(&aws);
}

#[test]
fn a_table_drops_itself_through_its_catalog() {
    let fake = S3TablesFake::start();
    let store = store();
    let catalog = catalog(&fake, &store);
    let desk = catalog
        .create_namespace("desk", &Properties::new())
        .expect("a namespace");
    let mut table = desk
        .create_table("quotes", &declared(), &Properties::new())
        .expect("a table");
    table
        .append_serie(rows(&[(2, 1)]), None)
        .expect("an append");

    // One `DeleteTable`, under no version token: the catalog that keeps the
    // table drops it, and the store is asked for nothing.
    fake.clear_requests();
    store.clear_requests();
    table.remove(true).expect("the table dropped");
    assert_eq!(
        fake.lines(),
        [format!("DELETE /tables/{LAKE_LABEL}/desk/quotes")]
    );
    assert_eq!(store.request_count(), 0);
    assert!(fake.table("lake", "desk", "quotes").is_none());
    let error = catalog.table("desk.quotes").expect_err("dropped");
    assert!(error.is_absent(), "{error}");

    // A table that is no longer there is already dropped.
    fake.clear_requests();
    table.remove(false).expect("already dropped");
    assert_eq!(fake.request_count(), 1);

    // The value that dropped it forgot the document it had read: the handle
    // asks the catalog whether the table is there, and it is not - and a
    // verb after the drop is that absence, never a commit to a table gone.
    fake.clear_requests();
    store.clear_requests();
    let error = table
        .append_serie(rows(&[(2, 1)]), None)
        .expect_err("dropped");
    assert!(error.is_absent(), "{error}");
    assert_eq!(fake.lines(), [metadata_location_line("quotes")]);
    assert_eq!(store.request_count(), 0, "{:?}", store.requests());
    fake.clear_requests();
    assert!(!Holder::from(table).exists());
    assert_eq!(fake.lines(), [metadata_location_line("quotes")]);
    assert_eq!(forbidden(&store), Vec::<String>::new());
}

/// A one-column row no format version before the second states less of.
fn plain() -> Field {
    DataType::from(
        StructType::from_fields([DataType::Int64.required_field("id")]).expect("a column"),
    )
    .required_field("row")
}

/// The ARN the service identifies the fake's table `lake.desk.<name>` by.
fn identified(fake: &S3TablesFake, name: &str) -> Arn {
    Arn::from_str(&fake.table("lake", "desk", name).expect("the table").arn).expect("a table's ARN")
}

/// The line `GetTable` addressed by `table` leaves in the log.
fn get_table_line(table: &Arn) -> String {
    format!(
        "GET /get-table?tableArn={}",
        crate::fake::encode(&table.to_string(), false)
    )
}

#[test]
fn a_table_opens_by_its_location_or_by_its_arn() {
    let fake = S3TablesFake::start();
    let store = store();
    let bucket = lake(&fake);
    let aws = scratch("open");
    let properties = stated(&fake, &store, &aws);
    let desk = catalog(&fake, &store)
        .create_namespace("desk", &Properties::new())
        .expect("a namespace");
    desk.create_table("quotes", &declared(), &Properties::new())
        .expect("a table")
        .append_serie(rows(&[(2, 1)]), None)
        .expect("an append");
    let arn = identified(&fake, "quotes");

    // A table's ARN is one `GetTable` addressed by it alone: its answer names
    // the namespace and the name, and the bucket's ARN is the table's own
    // less its identifier, so nothing lists the caller's buckets.
    fake.clear_requests();
    let table = IcebergTable::from_url(&arn, &properties).expect("the table");
    assert_eq!(ObjectValue::path(&table), ["lake", "desk", "quotes"]);
    assert_eq!(fake.lines(), [get_table_line(&arn)]);

    // That answer primed the pointer: the first read asks the service
    // nothing and reads the document it named, once.
    fake.clear_requests();
    store.clear_requests();
    assert_eq!(table.metadata_version().expect("its version"), 1);
    assert_eq!(fake.request_count(), 0, "{:?}", fake.lines());
    assert_eq!(fetched(&store).len(), 1, "{:?}", fetched(&store));
    assert_eq!(store.request_count(), 1, "{:?}", store.requests());
    assert_eq!(read(&Table::from(table)), [(0, 2, 1)]);

    // The same table by its location, the bucket's ARN known from the
    // account: one request, and the same rows.
    fake.clear_requests();
    let located = IcebergTable::from_url(
        url("s3tables://lake/desk/quotes"),
        &properties
            .clone()
            .with_property("account_id", crate::fake::ACCOUNT),
    )
    .expect("the table");
    assert_eq!(fake.lines(), [metadata_location_line("quotes")]);
    assert_eq!(read(&Table::from(located.clone())), [(0, 2, 1)]);
    assert_eq!(fake.request_count(), 1, "the read asked nothing more");

    // Who signs is the session's: the table keeps no identity property, so
    // it lists and prints no secret.
    let kept = ObjectValue::properties(&located).expect("its properties");
    assert!(
        kept.iter()
            .all(|(name, _)| !yggdryl::aws::Session::is_property(name)),
        "an identity property kept in {kept:?}"
    );
    assert!(!format!("{located:?}").contains(SECRET_KEY));

    // What a location names that is no table is refused by its kind, at no
    // request; an identifier the bucket has no table of is absent, at one.
    fake.clear_requests();
    for (location, kind) in [
        (bucket.clone().into_uri(), "catalog"),
        (url("s3tables://lake").into_uri(), "catalog"),
        (url("s3tables://lake/desk").into_uri(), "namespace"),
    ] {
        let error = IcebergTable::from_url(&location, &properties).expect_err("no table");
        let error = error.to_string();
        assert!(error.contains("$.url"), "{error}");
        assert!(error.contains(&format!("got the {kind} ")), "{error}");
    }
    assert_eq!(fake.request_count(), 0, "{:?}", fake.lines());
    let unknown = Arn::from_str(&format!(
        "{bucket}/table/00000000-0000-4000-8000-00000000beef"
    ))
    .expect("a table's ARN");
    let error = IcebergTable::from_url(&unknown, &properties).expect_err("no such table");
    assert!(error.is_absent(), "{error}");
    assert_eq!(fake.lines(), [get_table_line(&unknown)]);

    // An ARN below a bucket that is no table's is refused before a request.
    fake.clear_requests();
    let malformed = Arn::from_str(&format!("{bucket}/index/quotes")).expect("an ARN");
    let error = IcebergTable::from_url(&malformed, &properties).expect_err("no table's ARN");
    assert!(error.to_string().contains("table/<id>"), "{error}");
    assert_eq!(fake.request_count(), 0);
    let _ = std::fs::remove_dir_all(&aws);
}

#[test]
fn a_table_is_created_at_its_location_under_a_namespace_made_on_the_way() {
    let fake = S3TablesFake::start();
    let store = store();
    let arn = lake(&fake).to_string();
    let aws = scratch("create");
    let properties = stated(&fake, &store, &aws).with_property("warehouse", arn.as_str());
    let created = format!("PUT /tables/{LAKE_LABEL}/desk");
    let published = |name: &str| format!("PUT /tables/{LAKE_LABEL}/desk/{name}/metadata-location");

    // The namespace is not there: the refused creation, the namespace, then
    // the creation's own three requests and its one document.
    fake.clear_requests();
    store.clear_requests();
    let mut table = IcebergTable::create_from_url(
        url("s3tables://lake/desk/quotes"),
        &properties,
        None,
        declared(),
        None,
    )
    .expect("a table");
    assert_eq!(
        fake.lines(),
        [
            created.clone(),
            format!("PUT /namespaces/{LAKE_LABEL}"),
            created.clone(),
            metadata_location_line("quotes"),
            published("quotes"),
        ]
    );
    assert_eq!(fake.requests()[0].status, 404);
    let keys = written(&store);
    assert!(
        keys.len() == 1 && keys[0].ends_with(".metadata.json"),
        "{keys:?}"
    );
    // That one `PutObject` is the whole of what the store was asked.
    assert_eq!(store.request_count(), 1, "{:?}", store.requests());
    assert_eq!(ObjectValue::path(&table), ["lake", "desk", "quotes"]);
    // Neither a version nor a spec stated: the lowest version that states a
    // nanosecond instant, and the partition the schema declares.
    let metadata = table.metadata().expect("its document");
    assert_eq!(metadata.format_version(), FormatVersion::V3);
    assert_eq!(metadata.default_spec().expect("a spec").fields.len(), 1);
    assert!(!format!("{table:?}").contains(SECRET_KEY));

    // Under a namespace that is there, the three alone - at the version and
    // under the spec the create states - and the store's one `PutObject`.
    fake.clear_requests();
    store.clear_requests();
    let stated_layout = IcebergTable::create_from_url(
        url("s3tables://lake/desk/orders/"),
        &properties,
        Some(FormatVersion::V3),
        plain(),
        Some(PartitionSpec::unpartitioned()),
    )
    .expect("a table");
    assert_eq!(
        fake.lines(),
        [
            created.clone(),
            metadata_location_line("orders"),
            published("orders")
        ]
    );
    assert_eq!(written(&store).len(), 1, "{:?}", store.requests());
    assert_eq!(store.request_count(), 1, "{:?}", store.requests());
    assert_eq!(
        stated_layout
            .metadata()
            .expect("its document")
            .format_version(),
        FormatVersion::V3
    );

    // A table already there is a conflict by its warehouse path, at the one
    // request that says so.
    fake.clear_requests();
    let error = IcebergTable::create_from_url(
        url("s3tables://lake/desk/quotes"),
        &properties,
        None,
        declared(),
        None,
    )
    .expect_err("already there");
    assert!(error.is_conflict(), "{error}");
    assert!(error.to_string().contains("lake.desk.quotes"), "{error}");
    assert_eq!(fake.lines(), [created.as_str()]);

    // Opening or creating opens what is there and creates what is not.
    fake.clear_requests();
    let opened = IcebergTable::open_or_create_from_url(
        url("s3tables://lake/desk/quotes"),
        &properties,
        None,
        declared(),
        None,
    )
    .expect("the table");
    assert_eq!(fake.lines(), [metadata_location_line("quotes")]);
    assert_eq!(
        opened.metadata_location().expect("its location"),
        table.metadata_location().expect("its location")
    );
    fake.clear_requests();
    store.clear_requests();
    IcebergTable::open_or_create_from_url(
        url("s3tables://lake/desk/fills"),
        &properties,
        None,
        plain(),
        None,
    )
    .expect("a table");
    assert_eq!(
        fake.lines(),
        [
            metadata_location_line("fills"),
            created.clone(),
            metadata_location_line("fills"),
            published("fills"),
        ]
    );
    // The open's refused request touched no store: the create's one
    // `PutObject` is the whole of it.
    assert_eq!(written(&store).len(), 1, "{:?}", store.requests());
    assert_eq!(store.request_count(), 1, "{:?}", store.requests());

    // A table's ARN is opened where the bucket keeps the table and absent
    // where it does not: an identifier names nothing a create could make,
    // so its absence is the answer, at the one `GetTable` that says so.
    let orders = identified(&fake, "orders");
    fake.clear_requests();
    let by_arn = IcebergTable::open_or_create_from_url(&orders, &properties, None, plain(), None)
        .expect("the table");
    assert_eq!(ObjectValue::path(&by_arn), ["lake", "desk", "orders"]);
    assert_eq!(fake.lines(), [get_table_line(&orders)]);
    let unknown = Arn::from_str(&format!("{arn}/table/00000000-0000-4000-8000-00000000beef"))
        .expect("a table's ARN");
    fake.clear_requests();
    let error = IcebergTable::open_or_create_from_url(&unknown, &properties, None, plain(), None)
        .expect_err("no such table");
    assert!(error.is_absent(), "{error}");
    assert_eq!(fake.lines(), [get_table_line(&unknown)]);

    // A bucket or a namespace is no table, whichever door is asked: refused
    // by its kind, at no request.
    fake.clear_requests();
    for (location, kind) in [
        ("s3tables://lake", "catalog"),
        ("s3tables://lake/desk", "namespace"),
    ] {
        let error =
            IcebergTable::open_or_create_from_url(url(location), &properties, None, plain(), None)
                .expect_err("no table");
        let error = error.to_string();
        assert!(error.contains("$.url"), "{error}");
        assert!(error.contains(&format!("got the {kind} ")), "{error}");
    }
    assert_eq!(fake.request_count(), 0, "{:?}", fake.lines());

    // The table is written, reopened by its location and dropped as any
    // other the bucket keeps is.
    table
        .append_serie(rows(&[(QUARTER + 7, 3), (2, 1)]), None)
        .expect("an append");
    let mut reopened =
        IcebergTable::from_url(url("s3tables://lake/desk/quotes"), &properties).expect("the table");
    assert_eq!(
        read(&Table::from(reopened.clone())),
        [(0, 2, 1), (QUARTER, QUARTER + 7, 3)]
    );
    fake.clear_requests();
    reopened.remove(true).expect("the table dropped");
    assert_eq!(
        fake.lines(),
        [format!("DELETE /tables/{LAKE_LABEL}/desk/quotes")]
    );
    assert!(fake.table("lake", "desk", "quotes").is_none());

    // A create names a table by its namespace and its name: a bucket, a
    // namespace, a deeper path and a table's ARN - a table that exists -
    // are refused with no request, and so is a version no format has.
    fake.clear_requests();
    for refused in [
        url("s3tables://lake").into_uri(),
        url("s3tables://lake/desk").into_uri(),
        url("s3tables://lake/desk/quotes/extra").into_uri(),
        identified(&fake, "orders").into_uri(),
    ] {
        let error = IcebergTable::create_from_url(&refused, &properties, None, plain(), None)
            .expect_err("no table's location");
        assert!(error.to_string().contains("$.url"), "{refused}: {error}");
    }
    let error = IcebergTable::create_from_url(
        url("s3tables://lake/desk/refused"),
        &properties.clone().with_property("format-version", "7"),
        None,
        plain(),
        None,
    )
    .expect_err("no format version 7");
    assert!(
        error.to_string().contains("$.with.format-version"),
        "{error}"
    );
    assert_eq!(fake.request_count(), 0, "{:?}", fake.lines());
    assert_eq!(forbidden(&store), Vec::<String>::new());
    let _ = std::fs::remove_dir_all(&aws);
}

#[test]
fn a_create_under_a_bucket_that_is_not_there_is_the_services_refusal() {
    let fake = S3TablesFake::start();
    let store = store();
    let aws = scratch("no-bucket");
    // The account states the bucket's ARN, so nothing lists to find it.
    let properties = stated(&fake, &store, &aws).with_property("account_id", crate::fake::ACCOUNT);

    // The creation is refused, and the namespace made on the way is too:
    // what is missing is the bucket, and nothing is sent a third time.
    let error = IcebergTable::create_from_url(
        url("s3tables://nowhere/desk/quotes"),
        &properties,
        None,
        plain(),
        None,
    )
    .expect_err("no such bucket");
    assert_eq!(crate::mod_::refusal(&error), (404, "NotFoundException"));
    let lines = fake.lines();
    assert_eq!(lines.len(), 2, "{lines:?}");
    assert!(lines[0].starts_with("PUT /tables/"), "{lines:?}");
    assert!(lines[1].starts_with("PUT /namespaces/"), "{lines:?}");
    assert_eq!(store.request_count(), 0);
    let _ = std::fs::remove_dir_all(&aws);
}

#[test]
fn an_arn_is_read_as_the_arn_it_is_and_never_as_the_location_it_lowers_to() {
    let fake = S3TablesFake::start();
    let store = store();
    let bucket = lake(&fake);
    let aws = scratch("handle");
    let properties = stated(&fake, &store, &aws);
    catalog(&fake, &store)
        .create_namespace("desk", &Properties::new())
        .expect("a namespace")
        .create_table("quotes", &declared(), &Properties::new())
        .expect("a table");
    let arn = identified(&fake, "quotes");

    // A bucket's ARN is its catalog, the region and the account read off it:
    // no request, as an `Arn` or as the `Uri` it is.
    fake.clear_requests();
    for held in [
        Holder::from_url(&bucket, &properties).expect("the catalog"),
        Holder::from_url(bucket.clone().into_uri(), &properties).expect("the catalog"),
    ] {
        let Holder::Catalog(named) = &held else {
            panic!("expected a catalog, got {held:?}");
        };
        let Catalog::S3Tables(named) = named.as_ref() else {
            panic!("expected an S3 Tables catalog");
        };
        assert_eq!(named.bucket_arn().expect("the ARN"), &bucket);
    }
    assert_eq!(fake.request_count(), 0);

    // A table's ARN is the table, at the one `GetTable` it addresses.
    let held = Holder::from_url(&arn, &properties).expect("the table");
    let Holder::Table(table) = &held else {
        panic!("expected a table, got {held:?}");
    };
    assert_eq!(table.to_string(), "lake.desk.quotes");
    assert_eq!(fake.lines(), [get_table_line(&arn)]);

    // The location that ARN lowers to spells the identifier where a
    // namespace goes, and is no name of one: refused, at no request.
    fake.clear_requests();
    let lowered = arn.locator().expect("a location");
    let error = Holder::from_url(&lowered, &properties).expect_err("no namespace");
    assert!(error.to_string().contains("$.url"), "{error}");
    assert!(error.to_string().contains("read as the ARN"), "{error}");
    assert_eq!(fake.request_count(), 0);
    let _ = std::fs::remove_dir_all(&aws);
}

/// The same life against the live service, in a table bucket the operator
/// names; ignored, and gated on that name.
///
/// ```bash
/// YGGDRYL_S3TABLES_ARN=arn:aws:s3tables:<region>:<account>:bucket/<name> \
///   cargo test -p yggdryl --features s3tables --test s3tables catalog::live -- --ignored --nocapture
/// ```
///
/// Who signs is the default chain - `AWS_PROFILE` names a signed-in
/// profile - and the region is the ARN's. The namespace and the table it
/// creates are removed again however the run ended; the files the commits
/// wrote stay for the bucket's own unreferenced-file removal, since a
/// warehouse location takes no delete.
#[test]
#[ignore = "needs a table bucket: YGGDRYL_S3TABLES_ARN"]
fn live_a_table_bucket_commits_through_the_control_plane() {
    use yggdryl::aws::Session;
    use yggdryl::s3tables::S3Tables;

    let arn = std::env::var("YGGDRYL_S3TABLES_ARN")
        .expect("YGGDRYL_S3TABLES_ARN names the table bucket to run in");
    let arn = yggdryl::Arn::from_str(&arn).expect("a table bucket's ARN");
    let client = S3Tables::new(Session::new());
    let catalog = Catalog::from(
        S3TablesCatalog::new("live", client.clone(), arn.clone()).expect("a table bucket"),
    );
    let namespace = format!(
        "yggdryl_{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |elapsed| elapsed.as_millis())
    );
    let by_location = format!("{namespace}_by_location");
    let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let desk = catalog
            .create_namespace(&namespace, &Properties::new())
            .expect("a namespace");
        let mut table = desk
            .create_table("quotes", &declared(), &Properties::new())
            .expect("a table");
        table
            .append_serie(rows(&[(QUARTER + 7, 3), (2, 1)]), None)
            .expect("an append");
        table
            .overwrite_serie(rows(&[(QUARTER + 9, 9)]), None)
            .expect("an overwrite");
        let reopened = catalog
            .table(format!("{namespace}.quotes").as_str())
            .expect("the table");
        assert_eq!(read(&reopened), [(0, 2, 1), (QUARTER, QUARTER + 9, 9)]);

        // The location doors against the real control plane: a create by
        // location under a namespace the bucket does not hold, the table by
        // its own ARN, an identifier the bucket keeps no table of, a drop.
        let bucket = arn.bucket().expect("a table bucket's name");
        let stated = Properties::new().with_property("warehouse", arn.to_string());
        let mut located = IcebergTable::create_from_url(
            url(&format!("s3tables://{bucket}/{by_location}/quotes")),
            &stated,
            None,
            declared(),
            None,
        )
        .expect("a table under a namespace made on the way");
        located
            .append_serie(rows(&[(2, 1)]), None)
            .expect("an append");
        let table_arn = client
            .get_table(&arn, &by_location, "quotes")
            .expect("the table described")
            .arn()
            .clone();
        let by_arn = IcebergTable::from_url(&table_arn, &Properties::new()).expect("by its ARN");
        assert_eq!(read(&Table::from(by_arn)), [(0, 2, 1)]);
        let same = IcebergTable::open_or_create_from_url(
            &table_arn,
            &Properties::new(),
            None,
            declared(),
            None,
        )
        .expect("opened, never created");
        assert_eq!(
            ObjectValue::path(&same)[1..],
            [by_location.as_str(), "quotes"]
        );
        let unknown = Arn::from_str(&format!("{arn}/table/00000000-0000-4000-8000-00000000beef"))
            .expect("a table's ARN");
        let error =
            IcebergTable::from_url(&unknown, &Properties::new()).expect_err("no such table");
        assert!(error.is_absent(), "{error}");
        located.remove(true).expect("dropped through the catalog");
        let error = client
            .get_table(&arn, &by_location, "quotes")
            .expect_err("dropped");
        assert!(error.is_absent(), "{error}");
    }));
    let table = client.remove_table(&arn, &namespace, "quotes", None);
    let namespace = client.remove_namespace(&arn, &namespace);
    let located = client.remove_table(&arn, &by_location, "quotes", None);
    let by_location = client.remove_namespace(&arn, &by_location);
    if let Err(panic) = outcome {
        std::panic::resume_unwind(panic);
    }
    table.expect("the table removed");
    namespace.expect("the namespace removed");
    located.expect("the located table removed, or already dropped");
    by_location.expect("the namespace made on the way removed");
}

#[test]
fn a_dotted_path_descends_by_description_and_a_namespace_asked_for_is_one_request() {
    let fake = S3TablesFake::start();
    let store = store();
    let catalog = catalog(&fake, &store);
    let desk = catalog
        .create_namespace("desk", &Properties::new())
        .expect("a namespace");
    desk.create_table("quotes", &declared(), &Properties::new())
        .expect("a table");

    // The table by its dotted path: the namespace descended by description,
    // the table the one request - and its first read none, that answer
    // having primed the pointer.
    fake.clear_requests();
    let table = catalog.table("desk.quotes").expect("the table");
    assert_eq!(fake.lines(), [metadata_location_line("quotes")]);
    table.field().expect("its schema");
    assert_eq!(fake.request_count(), 1, "{:?}", fake.lines());

    // A namespace asked for by name is the existence question it always
    // was: one `GetNamespace`, and absent where the bucket holds none.
    fake.clear_requests();
    catalog.namespaces().get("desk").expect("the namespace");
    assert_eq!(fake.lines(), [get_namespace_line("desk")]);
    fake.clear_requests();
    assert!(!catalog.namespaces().contains("ghost").expect("an answer"));
    assert_eq!(fake.lines(), [get_namespace_line("ghost")]);

    // A table under a namespace the bucket does not hold is absent by its
    // path, at the table's one request and no `GetNamespace` before it.
    fake.clear_requests();
    let error = catalog
        .table("ghost.quotes")
        .expect_err("no such namespace");
    assert!(error.is_absent(), "{error}");
    assert!(error.to_string().contains("lake.ghost.quotes"), "{error}");
    assert_eq!(
        fake.lines(),
        [format!(
            "GET /tables/{LAKE_LABEL}/ghost/quotes/metadata-location"
        )]
    );
}

#[test]
fn every_store_of_a_bucket_signs_under_the_one_session_the_catalog_walked() {
    let fake = S3TablesFake::start();
    let store = store();
    let identity = Identity::start();
    identity.set_imds_role(Some("instance-role"));
    fake.accept_key(
        "ASIAINSTANCEROLE",
        "secret-of-instance-role",
        Some("token-of-instance-role"),
    );
    store.require_access_key(None);
    // A session that answers only through the instance metadata fake, so
    // every walk of the chain is a request the fake counts - and states no
    // region, as a session built from a profile or the environment states
    // none: the client's region is the bucket's, and the stores take it
    // from the catalog.
    let session = Session::new()
        .with_variables::<&str, &str>([])
        .with_directory(scratch("shared-store-session"))
        .with_metadata_endpoint(identity.endpoint());
    let catalog = Catalog::from(
        S3TablesCatalog::new(
            "lake",
            S3Tables::new(session)
                .with_region(REGION)
                .try_with_endpoint_url(fake.endpoint())
                .expect("the fake's endpoint"),
            lake(&fake),
        )
        .expect("a table bucket's ARN")
        .with_properties(store_properties(&store)),
    );
    let desk = catalog
        .create_namespace("desk", &Properties::new())
        .expect("a namespace");
    assert!(
        identity.request_count() > 0,
        "the control plane signed under the walked set"
    );

    // Three tables, each created through the control plane and written to
    // its own warehouse on the store, then read back. The first table's
    // store walks the chain once, for the session the bucket keeps in its
    // region; every table after it signs under that same session and the
    // fake hears nothing more - where a session forked per table would walk
    // the chain again for each.
    let mut walked = 0;
    for (count, name) in ["quotes", "orders", "fills"].into_iter().enumerate() {
        let mut table = desk
            .create_table(name, &declared(), &Properties::new())
            .expect("a table");
        table
            .append_serie(rows(&[(2, 1)]), None)
            .expect("an append");
        assert_eq!(read(&table), [(0, 2, 1)]);
        let reopened = catalog
            .table(format!("desk.{name}").as_str())
            .expect("the table");
        assert_eq!(read(&reopened), [(0, 2, 1)]);
        if count == 0 {
            walked = identity.request_count();
        } else {
            assert_eq!(
                identity.request_count(),
                walked,
                "table {name}: {:?}",
                identity.requests()
            );
        }
    }
}
