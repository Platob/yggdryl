//! `rust/src/s3tables/bucket.rs`: the table bucket verbs and the value they
//! answer - a creation, a description, a deletion, and the names and ARNs
//! refused before anything is sent.

use yggdryl::{DateTime64, Error, TimeUnit, Timezone};

use crate::fake::{ACCOUNT, S3TablesFake};
use crate::mod_::{LAKE_LABEL, arn, client, lake, refusal};

#[test]
fn a_bucket_is_created_and_its_arn_answered() {
    let fake = S3TablesFake::start();
    let tables = client(&fake);

    let created = tables.create_table_bucket("lake").expect("a bucket");
    assert_eq!(created.to_string(), fake.bucket_arn("lake"));
    assert_eq!(created.service(), "s3tables");
    assert_eq!(created.bucket(), Some("lake"));
    assert!(fake.has_bucket("lake"));

    let recorded = fake.requests();
    assert_eq!(recorded.len(), 1);
    assert_eq!(recorded[0].line(), "PUT /buckets");
    assert_eq!(recorded[0].body, r#"{"name":"lake"}"#);
}

#[test]
fn a_bucket_name_the_model_refuses_costs_no_request() {
    let fake = S3TablesFake::start();
    let tables = client(&fake);

    // 3 to 63 of the digits, the lower-case letters and the hyphen: each
    // refusal is at the first byte that breaks the rule.
    let long = "a".repeat(64);
    for (name, position) in [
        ("", 0),
        ("ab", 2),
        ("Lake", 0),
        ("under_score", 5),
        ("dot.ted", 3),
        ("espa\u{f1}ol", 4),
        (long.as_str(), 63),
    ] {
        let error = tables
            .create_table_bucket(name)
            .expect_err("a refused name");
        match &error {
            Error::Parse {
                target,
                position: at,
                reason,
            } => {
                assert_eq!(*target, "table bucket name");
                assert_eq!(*at, position, "{name:?}");
                assert!(
                    reason.contains("expected 3 to 63 of 0-9, a-z and '-'"),
                    "{reason}"
                );
            }
            other => panic!("expected a refused name for {name:?}, got {other:?}"),
        }
    }
    // The longest and the shortest names the model allows go out.
    tables
        .create_table_bucket(&"a".repeat(63))
        .expect("63 characters");
    tables.create_table_bucket("a-1").expect("3 characters");
    assert_eq!(fake.request_count(), 2);
}

#[test]
fn a_refusal_quotes_a_name_and_never_the_whole_of_a_long_one() {
    let fake = S3TablesFake::start();
    let tables = client(&fake);

    let message = tables
        .create_table_bucket(&"A".repeat(10_000))
        .expect_err("a refused name")
        .to_string();
    assert!(message.len() < 300, "{} bytes", message.len());
    assert_eq!(fake.request_count(), 0);
}

#[test]
fn a_bucket_that_is_already_there_is_a_conflict() {
    let fake = S3TablesFake::start();
    let tables = client(&fake);
    fake.seed_bucket("lake");

    let error = tables
        .create_table_bucket("lake")
        .expect_err("a bucket of that name");
    assert!(error.is_conflict());
    assert!(
        matches!(&error, Error::Conflict { expected, actual, path }
            if *expected == "table bucket" && *actual == "table bucket" && path.as_str() == "lake"),
        "{error:?}"
    );
    assert_eq!(fake.request_count(), 1, "the attempt is the probe");
}

#[test]
fn a_bucket_is_described_as_the_service_states_it() {
    let fake = S3TablesFake::start();
    let tables = client(&fake);
    let lake = lake(&fake);

    let bucket = tables.get_table_bucket(&lake).expect("a bucket");
    assert_eq!(bucket.arn(), &lake);
    assert_eq!(bucket.name(), "lake");
    assert_eq!(bucket.owner_account_id(), ACCOUNT);
    // The service's ISO 8601 date-time is the instant it spells, at the
    // resolution it spells it in.
    assert_eq!(
        bucket.created_at(),
        DateTime64::from_text("2026-01-01T00:00:01.000001Z", Timezone::UTC).expect("an instant")
    );
    assert_eq!(bucket.created_at().unit(), TimeUnit::Microsecond);
    assert!(!bucket.created_at().timezone().is_naive());
    assert_eq!(bucket.clone(), bucket);
    assert!(format!("{bucket:?}").contains("lake"));
}

#[test]
fn a_bucket_that_is_not_there_is_absent() {
    let fake = S3TablesFake::start();
    let tables = client(&fake);
    let absent = arn(&fake.bucket_arn("absent"));

    let error = tables
        .get_table_bucket(&absent)
        .expect_err("no such bucket");
    assert!(error.is_absent());
    assert!(
        matches!(&error, Error::Absent { expected, path }
            if *expected == "table bucket" && path.as_str() == absent.to_string()),
        "{error:?}"
    );
    assert_eq!(fake.request_count(), 1);
}

#[test]
fn removing_a_bucket_acts_once_and_an_absent_one_is_already_removed() {
    let fake = S3TablesFake::start();
    let tables = client(&fake);
    let lake = lake(&fake);

    tables
        .remove_table_bucket(&lake)
        .expect("the bucket deleted");
    assert!(!fake.has_bucket("lake"));
    // Deleted again: the service's NotFound is the state the caller asked
    // for, reached by the one request and no probe before it.
    tables
        .remove_table_bucket(&lake)
        .expect("nothing left to delete");
    assert_eq!(
        fake.requests()
            .iter()
            .map(|request| (request.line(), request.status))
            .collect::<Vec<_>>(),
        [
            (format!("DELETE /buckets/{LAKE_LABEL}"), 204),
            (format!("DELETE /buckets/{LAKE_LABEL}"), 404)
        ]
    );
}

#[test]
fn removing_a_bucket_that_holds_a_namespace_is_the_services_conflict() {
    let fake = S3TablesFake::start();
    let tables = client(&fake);
    let lake = lake(&fake);
    fake.seed_namespace("lake", "trial");

    let error = tables
        .remove_table_bucket(&lake)
        .expect_err("a bucket that is not empty");
    assert_eq!(refusal(&error), (409, "ConflictException"));
    // Nothing was created, so this is not the crate's conflict.
    assert!(!error.is_conflict());
    assert!(error.to_string().contains("not empty"), "{error}");
    assert!(fake.has_bucket("lake"));
}

#[test]
fn an_arn_that_names_no_table_bucket_is_refused_before_any_request() {
    let fake = S3TablesFake::start();
    let tables = client(&fake);

    for (text, position) in [
        // Another service: refused at its service field.
        ("arn:aws:s3:::lake", 8),
        ("arn:aws-cn:iam::123456789012:role/lake-reader", 11),
        // A table, not the bucket it is in: refused at its resource.
        (
            "arn:aws:s3tables:us-east-1:123456789012:bucket/lake/table/t-1",
            40,
        ),
        ("arn:aws:s3tables:us-east-1:123456789012:table/t-1", 40),
        ("arn:aws:s3tables:us-east-1:123456789012:bucket/", 40),
    ] {
        let named = arn(text);
        let refused = |error: Error| match error {
            Error::Parse {
                target,
                position: at,
                reason,
            } => {
                assert_eq!(target, "table bucket arn");
                assert_eq!(at, position, "{text}");
                assert!(
                    reason.contains("bucket/<name>") && reason.contains(text),
                    "{reason}"
                );
            }
            other => panic!("expected a refused ARN for {text}, got {other:?}"),
        };
        refused(
            tables
                .get_table_bucket(&named)
                .expect_err("no table bucket"),
        );
        refused(
            tables
                .remove_table_bucket(&named)
                .expect_err("no table bucket"),
        );
        refused(
            tables
                .create_namespace(&named, "trial")
                .expect_err("no table bucket"),
        );
        refused(
            tables
                .get_table(&named, "trial", "events")
                .expect_err("no table bucket"),
        );
        refused(
            tables
                .namespaces(&named)
                .next()
                .expect("one item")
                .expect_err("no table bucket"),
        );
    }
    assert_eq!(fake.request_count(), 0);
}
