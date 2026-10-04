//! `rust/src/s3tables/catalog.rs`: a table bucket as a warehouse catalog.
//!
//! The control plane is the fake every suite here runs on; each table's
//! warehouse location is a bucket of the fake object store the `s3` suites
//! run on, so both ends of a commit - the files and the pointer - are
//! requests a test reads back. What the store is never asked - a listing, a
//! delete, a version hint - is pinned from its log.

use std::sync::Arc;

use arrow_array::{Array, FixedSizeBinaryArray, RecordBatch, TimestampNanosecondArray};

use crate::fake::{ACCESS_KEY, REGION, S3TablesFake, SECRET_KEY};
use crate::mod_::{client, lake, scratch};
use crate::server::FakeS3;
use yggdryl::iceberg::FormatVersion;
use yggdryl::s3tables::S3TablesCatalog;
use yggdryl::{
    ArrowCastOptions, Catalog, CatalogValue, DataType, Field, IOBase, IOMedia, NamespaceValue,
    ObjectValue, Properties, Serie, StructType, Table, TableValue, TimeUnit, Timezone, Url,
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
    for record in table.read_serie(None).expect("a read") {
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
        .append_serie(rows(&[(QUARTER + 7, 3), (2, 1)]).into(), None)
        .expect("an append");
    table
        .append_serie(rows(&[(QUARTER + 1, 4), (5, 2)]).into(), None)
        .expect("an append");
    table
        .overwrite_serie(rows(&[(QUARTER + 9, 9)]).into(), None)
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

    // Reopened through the catalog: one request a level - the namespace,
    // then where the table's document is - and the same rows read through
    // the document the service names.
    fake.clear_requests();
    let reopened = catalog.table("desk.quotes").expect("the table");
    assert_eq!(fake.request_count(), 2, "{:?}", fake.lines());
    assert_eq!(read(&reopened), read(&table));
    assert_eq!(
        reopened
            .field()
            .expect("its schema")
            .get_metadata("PARTITION:by"),
        Some(r#"["part"]"#)
    );

    // The catalog keeps the table: its own listing and removal are refused
    // rather than reaching the store, and the service still names it.
    let mut kept = reopened;
    let error = kept.remove(true).expect_err("kept by the catalog");
    assert!(
        error.to_string().contains("drop it through that catalog"),
        "{error}"
    );
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
        .append_serie(rows(&[(1, 1)]).into(), None)
        .expect("an append");

    // An overwrite planned against version 0 is refused rather than
    // replacing what the first wrote...
    let error = second
        .overwrite_serie(rows(&[(2, 2)]).into(), None)
        .expect_err("a stale token");
    assert!(error.is_conflict(), "{error}");
    assert!(error.to_string().contains("last saw version 1"), "{error}");

    // ...while an append reads where the table stands and applies again.
    second
        .append_serie(rows(&[(3, 3)]).into(), None)
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

    // The catalog prints none of the secrets it was stated.
    let shown = format!("{catalog:?}");
    assert!(!shown.contains(SECRET_KEY), "a secret in {shown}");
    assert!(shown.contains("<redacted>"), "{shown}");

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
    let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let desk = catalog
            .create_namespace(&namespace, &Properties::new())
            .expect("a namespace");
        let mut table = desk
            .create_table("quotes", &declared(), &Properties::new())
            .expect("a table");
        table
            .append_serie(rows(&[(QUARTER + 7, 3), (2, 1)]).into(), None)
            .expect("an append");
        table
            .overwrite_serie(rows(&[(QUARTER + 9, 9)]).into(), None)
            .expect("an overwrite");
        let reopened = catalog
            .table(format!("{namespace}.quotes").as_str())
            .expect("the table");
        assert_eq!(read(&reopened), [(0, 2, 1), (QUARTER, QUARTER + 9, 9)]);
    }));
    let table = client.remove_table(&arn, &namespace, "quotes", None);
    let namespace = client.remove_namespace(&arn, &namespace);
    if let Err(panic) = outcome {
        std::panic::resume_unwind(panic);
    }
    table.expect("the table removed");
    namespace.expect("the namespace removed");
}
