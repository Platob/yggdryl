//! `rust/src/s3tables/listing.rs`: the three listings - one request per
//! page, asked for when the page before it is drained, and nothing after
//! the first error.

use serde_json::json;
use yggdryl::Error;

use crate::fake::{ACCOUNT, S3TablesFake};
use crate::mod_::{LAKE_LABEL, arn, client, lake, refusal};

#[test]
fn a_listing_asks_for_a_page_only_when_the_one_before_it_is_drained() {
    let fake = S3TablesFake::start();
    let tables = client(&fake);
    for name in ["bucket-a", "bucket-b", "bucket-c", "bucket-d", "bucket-e"] {
        fake.seed_bucket(name);
    }
    fake.set_page_size(2);

    let mut buckets = tables.table_buckets();
    assert_eq!(fake.request_count(), 0, "building a listing sends nothing");

    let mut names = Vec::new();
    // What the fake has been asked after each entry is handed over: the
    // second page is not requested while the first still holds an entry.
    let mut asked = Vec::new();
    for bucket in buckets.by_ref() {
        names.push(bucket.expect("a bucket").name().to_owned());
        asked.push(fake.request_count());
    }
    assert_eq!(
        names,
        ["bucket-a", "bucket-b", "bucket-c", "bucket-d", "bucket-e"]
    );
    assert_eq!(asked, [1, 1, 2, 2, 3], "one request per page of two");

    // Drained: asking again costs nothing and answers nothing.
    assert!(buckets.next().is_none());
    assert!(buckets.next().is_none());
    assert_eq!(fake.request_count(), 3);
}

#[test]
fn a_page_is_continued_by_the_token_the_one_before_it_answered() {
    let fake = S3TablesFake::start();
    let tables = client(&fake);
    for name in ["bucket-a", "bucket-b", "bucket-c"] {
        fake.seed_bucket(name);
    }
    fake.set_page_size(2);

    assert_eq!(tables.table_buckets().count(), 3);
    let recorded = fake.requests();
    assert_eq!(recorded.len(), 2);
    // The first page states the page size and no token.
    assert_eq!(recorded[0].target, "/buckets?maxBuckets=250");
    // The second hands the token back as it was answered: the service's
    // tokens are not URL-safe, so its `/`, `+` and `=` are escaped on the
    // wire and arrive as they left.
    assert_eq!(
        recorded[1].target,
        "/buckets?continuationToken=after%2Fbucket-b%2B%3D&maxBuckets=250"
    );
    assert_eq!(
        recorded[1].query("continuationToken"),
        Some("after/bucket-b+=")
    );
}

#[test]
fn the_buckets_of_a_listing_are_the_values_a_description_answers() {
    let fake = S3TablesFake::start();
    let tables = client(&fake);
    let lake = lake(&fake);

    let listed: Vec<_> = tables
        .table_buckets()
        .collect::<yggdryl::Result<_>>()
        .expect("one page");
    assert_eq!(listed, [tables.get_table_bucket(&lake).expect("a bucket")]);
    assert_eq!(listed[0].owner_account_id(), ACCOUNT);
}

#[test]
fn an_empty_listing_is_one_request_and_no_entry() {
    let fake = S3TablesFake::start();
    let tables = client(&fake);
    let lake = lake(&fake);

    assert_eq!(tables.namespaces(&lake).count(), 0);
    assert_eq!(tables.tables(&lake, None).count(), 0);
    assert_eq!(
        fake.lines(),
        [
            format!("GET /namespaces/{LAKE_LABEL}?maxNamespaces=250"),
            format!("GET /tables/{LAKE_LABEL}?maxTables=250"),
        ]
    );
}

#[test]
fn namespaces_are_listed_a_page_a_request() {
    let fake = S3TablesFake::start();
    let tables = client(&fake);
    let lake = lake(&fake);
    for namespace in ["desk", "risk", "trial"] {
        fake.seed_namespace("lake", namespace);
    }
    fake.set_page_size(1);

    let listed: Vec<_> = tables
        .namespaces(&lake)
        .collect::<yggdryl::Result<_>>()
        .expect("three pages");
    assert_eq!(
        listed
            .iter()
            .map(yggdryl::s3tables::NamespaceSummary::name)
            .collect::<Vec<_>>(),
        ["desk", "risk", "trial"]
    );
    assert_eq!(
        listed[2],
        tables.get_namespace(&lake, "trial").expect("a namespace")
    );
    assert_eq!(
        fake.lines()[..3],
        [
            format!("GET /namespaces/{LAKE_LABEL}?maxNamespaces=250"),
            format!(
                "GET /namespaces/{LAKE_LABEL}?continuationToken=after%2Fdesk%2B%3D&maxNamespaces=250"
            ),
            format!(
                "GET /namespaces/{LAKE_LABEL}?continuationToken=after%2Frisk%2B%3D&maxNamespaces=250"
            ),
        ]
    );
}

#[test]
fn tables_are_listed_across_a_bucket_or_within_one_namespace() {
    let fake = S3TablesFake::start();
    let tables = client(&fake);
    let lake = lake(&fake);
    let events = fake.seed_table("lake", "trial", "events");
    fake.seed_table("lake", "trial", "fills");
    fake.seed_table("lake", "desk", "orders");
    fake.set_page_size(2);

    // The whole bucket: three tables, two pages.
    fake.clear_requests();
    let all: Vec<_> = tables
        .tables(&lake, None)
        .collect::<yggdryl::Result<_>>()
        .expect("two pages");
    assert_eq!(
        all.iter()
            .map(|table| (table.namespace(), table.name()))
            .collect::<Vec<_>>(),
        [("desk", "orders"), ("trial", "events"), ("trial", "fills")]
    );
    assert_eq!(fake.request_count(), 2);

    // A summary names the table and says when it changed; the token and the
    // locations are a description's.
    let summary = &all[1];
    assert_eq!(summary.arn(), &arn(&events.arn));
    assert_eq!(summary.created_at(), summary.modified_at());
    let described = tables.get_table(&lake, "trial", "events").expect("a table");
    assert_eq!(summary.arn(), described.arn());
    assert_eq!(summary.created_at(), described.created_at());
    assert_eq!(summary.clone(), *summary);

    // One namespace: the filter rides in the query of every page.
    fake.set_page_size(1);
    fake.clear_requests();
    let trial: Vec<_> = tables
        .tables(&lake, Some("trial"))
        .map(|table| table.expect("a table").name().to_owned())
        .collect();
    assert_eq!(trial, ["events", "fills"]);
    assert_eq!(
        fake.lines(),
        [
            format!("GET /tables/{LAKE_LABEL}?maxTables=250&namespace=trial"),
            format!(
                "GET /tables/{LAKE_LABEL}?continuationToken=after%2Ftrial.events%2B%3D&maxTables=250&namespace=trial"
            ),
        ]
    );
}

#[test]
fn a_listing_ends_at_its_first_error() {
    let fake = S3TablesFake::start();
    let tables = client(&fake);
    for name in ["bucket-a", "bucket-b", "bucket-c"] {
        fake.seed_bucket(name);
    }
    fake.set_page_size(1);

    let mut buckets = tables.table_buckets();
    assert_eq!(
        buckets
            .next()
            .expect("an entry")
            .expect("the first page")
            .name(),
        "bucket-a"
    );
    // The second page is refused: the refusal is handed over once, and the
    // walk is over - the third page is never asked for.
    fake.refuse_next(403, "ForbiddenException", "not yours", 1);
    let error = buckets
        .next()
        .expect("the refusal")
        .expect_err("the second page refused");
    assert_eq!(refusal(&error), (403, "ForbiddenException"));
    assert!(
        matches!(&error, Error::Remote { operation, path, .. }
            if *operation == "ListTableBuckets" && path.as_str() == "table buckets"),
        "{error:?}"
    );
    assert!(buckets.next().is_none());
    assert!(buckets.next().is_none());
    assert_eq!(fake.request_count(), 2);
    assert_eq!(buckets.size_hint(), (0, Some(0)));
}

/// One bucket as a listing states it.
fn listed_bucket(name: &str) -> serde_json::Value {
    json!({
        "arn": format!("arn:aws:s3tables:us-east-1:{ACCOUNT}:bucket/{name}"),
        "name": name,
        "ownerAccountId": ACCOUNT,
        "createdAt": "2026-01-01T00:00:01Z",
    })
}

#[test]
fn an_entry_that_does_not_read_ends_the_walk_in_its_place() {
    let fake = S3TablesFake::start();
    let tables = client(&fake);

    // The entry before it is handed over; the one after it is not.
    fake.answer_next(
        200,
        json!({"tableBuckets": [
            listed_bucket("bucket-a"),
            {"arn": "not an arn", "name": "bucket-b"},
            listed_bucket("bucket-c"),
        ]}),
    );
    let walked: Vec<_> = tables.table_buckets().collect();
    assert_eq!(walked.len(), 2, "{walked:?}");
    assert_eq!(
        walked[0].as_ref().expect("the first entry").name(),
        "bucket-a"
    );
    let error = walked[1]
        .as_ref()
        .expect_err("the entry that does not read");
    assert!(
        matches!(error, Error::Remote { code, .. } if code.as_str() == "MalformedAnswer"),
        "{error:?}"
    );
    assert_eq!(fake.request_count(), 1);
}

#[test]
fn a_continuation_token_answered_twice_is_refused_rather_than_ending_the_walk() {
    let fake = S3TablesFake::start();
    let tables = client(&fake);
    for name in ["bucket-a", "bucket-b"] {
        fake.seed_bucket(name);
    }
    fake.set_page_size(1);

    let mut buckets = tables.table_buckets();
    assert_eq!(
        buckets.next().expect("an entry").expect("a bucket").name(),
        "bucket-a"
    );
    // The second page hands back the token it was asked with: walking it
    // would loop, and stopping would call a partial listing whole.
    fake.answer_next(
        200,
        json!({"tableBuckets": [listed_bucket("bucket-b")], "continuationToken": "after/bucket-a+="}),
    );
    assert_eq!(
        buckets.next().expect("an entry").expect("a bucket").name(),
        "bucket-b"
    );
    let error = buckets
        .next()
        .expect("the refusal")
        .expect_err("a token answered twice");
    assert!(
        matches!(&error, Error::Remote { code, message, .. }
            if code.as_str() == "MalformedAnswer" && message.contains("continuation token")),
        "{error:?}"
    );
    assert!(buckets.next().is_none());
    assert_eq!(fake.request_count(), 2);
}

#[test]
fn a_listing_that_cannot_be_addressed_is_one_refusal_and_no_request() {
    let fake = S3TablesFake::start();
    let tables = client(&fake);
    let object_bucket = arn("arn:aws:s3:::lake");

    let mut namespaces = tables.namespaces(&object_bucket);
    assert!(matches!(
        namespaces.next(),
        Some(Err(Error::Parse { target, .. })) if target == "table bucket arn"
    ));
    assert!(namespaces.next().is_none());

    let mut listed = tables.tables(&object_bucket, None);
    assert!(matches!(listed.next(), Some(Err(Error::Parse { .. }))));
    assert!(listed.next().is_none());
    assert_eq!(fake.request_count(), 0);

    // A listing under a namespace that is not there is the service's own
    // 404: what is missing is not a table.
    let lake = lake(&fake);
    let error = tables
        .tables(&lake, Some("absent"))
        .next()
        .expect("one item")
        .expect_err("no such namespace");
    assert_eq!(refusal(&error), (404, "NotFoundException"));
}
