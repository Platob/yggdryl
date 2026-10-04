//! `rust/src/s3tables/mod.rs`: the request every verb costs, which the
//! module's own table states, and a table's whole life against the fake,
//! beside the helpers every suite here shares.

use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};

use yggdryl::aws::{Credentials, Session};
use yggdryl::s3tables::S3Tables;
use yggdryl::{Arn, DataType, Error, Field, StructType, TimeUnit, Timezone, Url};

use crate::fake::{ACCESS_KEY, REGION, S3TablesFake, SECRET_KEY};

/// The label the table bucket `lake` of the fake is addressed by: its ARN,
/// percent-encoded once.
pub const LAKE_LABEL: &str = "arn%3Aaws%3As3tables%3Aus-east-1%3A123456789012%3Abucket%2Flake";

/// A session that consults nothing outside the test - no variable, no file,
/// no metadata service - signs with the pair the fake accepts, and is in
/// the fake's region.
pub fn session() -> Session {
    Session::new()
        .with_environment(false)
        .with_credentials(Credentials::new(ACCESS_KEY, SECRET_KEY))
        .with_region(REGION)
}

/// A client of `fake`, signing as [`session`] answers.
pub fn client(fake: &S3TablesFake) -> S3Tables {
    S3Tables::new(session())
        .try_with_endpoint_url(fake.endpoint())
        .expect("the fake's endpoint")
}

/// `text` as the ARN it spells.
pub fn arn(text: &str) -> Arn {
    Arn::from_str(text).expect("an ARN")
}

/// The table bucket `lake`, put in the fake without a request.
pub fn lake(fake: &S3TablesFake) -> Arn {
    arn(&fake.seed_bucket("lake"))
}

/// A directory of this test's own under the platform's temporary one, empty
/// when handed over, for the `~/.aws` a session is pointed at.
pub fn scratch(name: &str) -> PathBuf {
    static COUNTER: AtomicUsize = AtomicUsize::new(0);
    let unique = COUNTER.fetch_add(1, Ordering::Relaxed);
    let path = std::env::temp_dir().join(format!(
        "yggdryl-s3tables-{name}-{}-{unique}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&path);
    std::fs::create_dir_all(&path).expect("a scratch directory");
    path
}

/// A three-column row: a required `id`, a `symbol`, and an instant.
pub fn schema() -> Field {
    DataType::from(
        StructType::from_fields([
            DataType::Int64.required_field("id"),
            DataType::utf8().nullable_field("symbol"),
            DataType::datetime64(TimeUnit::Microsecond, Timezone::UTC)
                .expect("a timestamp")
                .nullable_field("ts"),
        ])
        .expect("three columns"),
    )
    .required_field("row")
}

/// The service's refusal an error carries: its status and its error type.
///
/// # Panics
///
/// When `error` is not a refusal of the service.
pub fn refusal(error: &Error) -> (u16, &str) {
    match error {
        Error::Remote {
            service,
            status,
            code,
            ..
        } => {
            assert_eq!(*service, "s3tables");
            (*status, code.as_str())
        }
        other => panic!("expected the service's refusal, got {other:?}"),
    }
}

#[test]
fn every_verb_is_exactly_one_request() {
    let fake = S3TablesFake::start();
    let tables = client(&fake);
    let lake = lake(&fake);
    let seeded = fake.seed_table("lake", "trial", "events");
    let staging = arn(&fake.bucket_arn("staging"));

    // Each verb is run alone between two readings of the fake's log, so the
    // line it leaves is the whole of what it sent.
    let sent = |verb: &dyn Fn()| {
        fake.clear_requests();
        verb();
        fake.lines()
    };

    assert_eq!(
        sent(&|| drop(tables.create_table_bucket("staging").expect("a bucket"))),
        ["PUT /buckets"]
    );
    assert_eq!(
        sent(&|| drop(tables.get_table_bucket(&lake).expect("a bucket"))),
        [format!("GET /buckets/{LAKE_LABEL}")]
    );
    assert_eq!(
        sent(&|| assert_eq!(tables.table_buckets().count(), 2)),
        ["GET /buckets?maxBuckets=250"]
    );
    assert_eq!(
        sent(&|| tables.create_namespace(&lake, "desk").expect("a namespace")),
        [format!("PUT /namespaces/{LAKE_LABEL}")]
    );
    assert_eq!(
        sent(&|| drop(tables.get_namespace(&lake, "trial").expect("a namespace"))),
        [format!("GET /namespaces/{LAKE_LABEL}/trial")]
    );
    assert_eq!(
        sent(&|| assert_eq!(tables.namespaces(&lake).count(), 2)),
        [format!("GET /namespaces/{LAKE_LABEL}?maxNamespaces=250")]
    );
    assert_eq!(
        sent(&|| drop(
            tables
                .create_table(&lake, "desk", "orders", None)
                .expect("a table")
        )),
        [format!("PUT /tables/{LAKE_LABEL}/desk")]
    );
    assert_eq!(
        sent(&|| drop(tables.get_table(&lake, "trial", "events").expect("a table"))),
        [format!(
            "GET /get-table?name=events&namespace=trial&tableBucketARN={LAKE_LABEL}"
        )]
    );
    assert_eq!(
        sent(&|| drop(tables.get_table_by_arn(&arn(&seeded.arn)).expect("a table"))),
        [format!(
            "GET /get-table?tableArn={}",
            crate::fake::encode(&seeded.arn, false)
        )]
    );
    assert_eq!(
        sent(&|| assert_eq!(tables.tables(&lake, Some("trial")).count(), 1)),
        [format!(
            "GET /tables/{LAKE_LABEL}?maxTables=250&namespace=trial"
        )]
    );
    assert_eq!(
        sent(&|| assert_eq!(tables.tables(&lake, None).count(), 2)),
        [format!("GET /tables/{LAKE_LABEL}?maxTables=250")]
    );
    assert_eq!(
        sent(&|| drop(
            tables
                .get_table_metadata_location(&lake, "trial", "events")
                .expect("a location")
        )),
        [format!(
            "GET /tables/{LAKE_LABEL}/trial/events/metadata-location"
        )]
    );
    let next = Url::from_str(&format!(
        "{}/metadata/00001-next.metadata.json",
        seeded.warehouse_location
    ))
    .expect("a location");
    assert_eq!(
        sent(&|| drop(
            tables
                .update_table_metadata_location(
                    &lake,
                    "trial",
                    "events",
                    &seeded.version_token,
                    &next
                )
                .expect("a commit")
        )),
        [format!(
            "PUT /tables/{LAKE_LABEL}/trial/events/metadata-location"
        )]
    );
    assert_eq!(
        sent(&|| tables
            .rename_table(&lake, "trial", "events", None, Some("events_renamed"), None)
            .expect("a rename")),
        [format!("PUT /tables/{LAKE_LABEL}/trial/events/rename")]
    );
    assert_eq!(
        sent(&|| tables
            .remove_table(&lake, "trial", "events_renamed", None)
            .expect("a deletion")),
        [format!("DELETE /tables/{LAKE_LABEL}/trial/events_renamed")]
    );
    let orders = fake.table("lake", "desk", "orders").expect("the table");
    assert_eq!(
        sent(&|| tables
            .remove_table(&lake, "desk", "orders", Some(&orders.version_token))
            .expect("a deletion")),
        [format!(
            "DELETE /tables/{LAKE_LABEL}/desk/orders?versionToken={}",
            orders.version_token
        )]
    );
    assert_eq!(
        sent(&|| tables.remove_namespace(&lake, "trial").expect("a deletion")),
        [format!("DELETE /namespaces/{LAKE_LABEL}/trial")]
    );
    assert_eq!(
        sent(&|| tables.remove_table_bucket(&staging).expect("a deletion")),
        [format!(
            "DELETE /buckets/{}",
            LAKE_LABEL.replace("lake", "staging")
        )]
    );
}

#[test]
fn building_a_client_a_listing_or_a_value_sends_nothing() {
    let fake = S3TablesFake::start();
    let tables = client(&fake);
    let lake = lake(&fake);

    let _buckets = tables.table_buckets();
    let _namespaces = tables.namespaces(&lake);
    let _tables = tables.tables(&lake, Some("trial"));
    let _clone = tables.clone().with_region("eu-west-3");
    assert_eq!(
        tables.region_of(Some(&lake)).expect("a region"),
        "us-east-1"
    );
    assert_eq!(
        tables.endpoint_url("us-east-1").expect("an endpoint"),
        fake.endpoint()
    );
    assert_eq!(fake.request_count(), 0);
}

#[test]
fn a_client_is_shared_and_a_listing_is_moved_between_threads() {
    use yggdryl::s3tables::{NamespaceSummaries, TableBuckets, TableSummaries};

    // A listing owns what it needs, the client included: it borrows nothing,
    // so a caller holds it, or drains it on another thread.
    fn shared<T: Clone + Send + Sync + 'static>() {}
    fn moved<T: Iterator + Send + 'static>() {}
    shared::<S3Tables>();
    moved::<TableBuckets>();
    moved::<NamespaceSummaries>();
    moved::<TableSummaries>();

    let fake = S3TablesFake::start();
    fake.seed_bucket("lake");
    let listing = client(&fake).table_buckets();
    let names: Vec<String> = std::thread::spawn(move || {
        listing
            .map(|bucket| bucket.expect("a bucket").name().to_owned())
            .collect()
    })
    .join()
    .expect("the walk");
    assert_eq!(names, ["lake"]);
}

#[test]
fn a_table_lives_its_whole_life_in_the_catalog() {
    let fake = S3TablesFake::start();
    let tables = client(&fake);

    // A bucket, a namespace, and a table with a schema.
    let lake = tables.create_table_bucket("lake").expect("a bucket");
    assert_eq!(lake.to_string(), fake.bucket_arn("lake"));
    tables
        .create_namespace(&lake, "trial")
        .expect("a namespace");
    let created = tables
        .create_table(&lake, "trial", "events", Some(&schema()))
        .expect("a table");
    let held = fake.table("lake", "trial", "events").expect("the table");
    assert_eq!(created.arn().to_string(), held.arn);
    assert_eq!(created.version_token(), held.version_token);
    assert_eq!(created.arn().bucket(), Some("lake"));

    // The service wrote the first metadata file, and says where.
    let first = tables
        .get_table_metadata_location(&lake, "trial", "events")
        .expect("a location");
    assert_eq!(first.version_token(), created.version_token());
    assert_eq!(
        first.warehouse_location().to_string(),
        held.warehouse_location
    );
    assert_eq!(
        first.metadata_location().map(ToString::to_string),
        held.metadata_location
    );

    // A commit names the next metadata file under the token just read, and
    // the token moves.
    let next = Url::from_str(&format!(
        "{}/metadata/00001-commit.metadata.json",
        first.warehouse_location()
    ))
    .expect("a location");
    let committed = tables
        .update_table_metadata_location(&lake, "trial", "events", first.version_token(), &next)
        .expect("a commit");
    assert_eq!(committed.arn(), created.arn());
    assert_ne!(committed.version_token(), first.version_token());
    let second = tables
        .get_table_metadata_location(&lake, "trial", "events")
        .expect("a location");
    assert_eq!(second.metadata_location(), Some(&next));
    assert_eq!(second.version_token(), committed.version_token());

    // A rename keeps the table and moves its token.
    tables
        .rename_table(
            &lake,
            "trial",
            "events",
            None,
            Some("events_renamed"),
            Some(second.version_token()),
        )
        .expect("a rename");
    assert!(fake.table("lake", "trial", "events").is_none());
    let renamed = tables
        .get_table(&lake, "trial", "events_renamed")
        .expect("the renamed table");
    assert_eq!(renamed.arn(), created.arn());
    assert_eq!(renamed.metadata_location(), Some(&next));
    assert_ne!(renamed.version_token(), second.version_token());

    // Emptied from the bottom up, everything goes.
    tables
        .remove_table(
            &lake,
            "trial",
            "events_renamed",
            Some(renamed.version_token()),
        )
        .expect("the table deleted");
    tables
        .remove_namespace(&lake, "trial")
        .expect("the namespace deleted");
    tables
        .remove_table_bucket(&lake)
        .expect("the bucket deleted");
    assert!(!fake.has_bucket("lake"));
    assert_eq!(tables.table_buckets().count(), 0);
}
