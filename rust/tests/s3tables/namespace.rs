//! `rust/src/s3tables/namespace.rs`: the namespace verbs and the value they
//! answer.

use yggdryl::{DateTime64, Error, Timezone};

use crate::fake::{ACCOUNT, S3TablesFake};
use crate::mod_::{LAKE_LABEL, arn, client, lake, refusal};

#[test]
fn a_namespace_is_created_under_its_table_bucket() {
    let fake = S3TablesFake::start();
    let tables = client(&fake);
    let lake = lake(&fake);

    tables
        .create_namespace(&lake, "trial")
        .expect("a namespace");
    assert!(fake.has_namespace("lake", "trial"));

    let recorded = fake.requests();
    assert_eq!(recorded.len(), 1);
    assert_eq!(recorded[0].line(), format!("PUT /namespaces/{LAKE_LABEL}"));
    // The model spells a namespace as a list of names, of exactly one.
    assert_eq!(recorded[0].body, r#"{"namespace":["trial"]}"#);
}

#[test]
fn a_namespace_name_the_model_refuses_costs_no_request() {
    let fake = S3TablesFake::start();
    let tables = client(&fake);
    let lake = lake(&fake);

    let long = "a".repeat(256);
    for (name, position) in [
        ("", 0),
        ("Trial", 0),
        ("with-hyphen", 4),
        ("two.levels", 3),
        (long.as_str(), 255),
    ] {
        let refused = |error: Error| match error {
            Error::Parse {
                target,
                position: at,
                reason,
            } => {
                assert_eq!(target, "namespace name");
                assert_eq!(at, position, "{name:?}");
                assert!(
                    reason.contains("expected 1 to 255 of 0-9, a-z and '_'"),
                    "{reason}"
                );
            }
            other => panic!("expected a refused name for {name:?}, got {other:?}"),
        };
        refused(
            tables
                .create_namespace(&lake, name)
                .expect_err("a refused name"),
        );
        refused(
            tables
                .get_namespace(&lake, name)
                .expect_err("a refused name"),
        );
        refused(
            tables
                .remove_namespace(&lake, name)
                .expect_err("a refused name"),
        );
        refused(
            tables
                .tables(&lake, Some(name))
                .next()
                .expect("one item")
                .expect_err("a refused name"),
        );
    }
    assert_eq!(fake.request_count(), 0);

    // The longest name the model allows goes out, as one path label.
    let longest = "n".repeat(255);
    tables
        .create_namespace(&lake, &longest)
        .expect("255 characters");
    tables
        .get_namespace(&lake, &longest)
        .expect("the namespace");
    assert_eq!(
        fake.requests()[1].target,
        format!("/namespaces/{LAKE_LABEL}/{longest}")
    );
}

#[test]
fn a_namespace_that_is_already_there_is_a_conflict() {
    let fake = S3TablesFake::start();
    let tables = client(&fake);
    let lake = lake(&fake);
    fake.seed_namespace("lake", "trial");

    let error = tables
        .create_namespace(&lake, "trial")
        .expect_err("a namespace of that name");
    assert!(error.is_conflict());
    assert!(
        matches!(&error, Error::Conflict { expected, path, .. }
            if *expected == "namespace" && path.as_str() == format!("{lake}/trial")),
        "{error:?}"
    );
    assert_eq!(fake.request_count(), 1);
}

#[test]
fn a_namespace_under_a_bucket_that_is_not_there_is_the_services_not_found() {
    let fake = S3TablesFake::start();
    let tables = client(&fake);
    let absent = arn(&fake.bucket_arn("absent"));

    // What is missing is the bucket, not the namespace addressed, so the
    // service's own words stand.
    let error = tables
        .create_namespace(&absent, "trial")
        .expect_err("no such bucket");
    assert_eq!(refusal(&error), (404, "NotFoundException"));
    assert!(!error.is_absent());
    let error = tables
        .namespaces(&absent)
        .next()
        .expect("one item")
        .expect_err("no such bucket");
    assert_eq!(refusal(&error), (404, "NotFoundException"));
}

#[test]
fn a_namespace_is_described_as_the_service_states_it() {
    let fake = S3TablesFake::start();
    let tables = client(&fake);
    let lake = lake(&fake);
    fake.seed_namespace("lake", "trial");

    let namespace = tables.get_namespace(&lake, "trial").expect("a namespace");
    assert_eq!(namespace.name(), "trial");
    assert_eq!(namespace.created_by(), ACCOUNT);
    assert_eq!(namespace.owner_account_id(), ACCOUNT);
    assert_eq!(
        namespace.created_at(),
        DateTime64::from_text("2026-01-01T00:00:03.000003Z", Timezone::UTC).expect("an instant")
    );
    assert_eq!(namespace.clone(), namespace);
}

#[test]
fn a_namespace_that_is_not_there_is_absent() {
    let fake = S3TablesFake::start();
    let tables = client(&fake);
    let lake = lake(&fake);

    let error = tables
        .get_namespace(&lake, "absent")
        .expect_err("no such namespace");
    assert!(error.is_absent());
    assert!(
        matches!(&error, Error::Absent { expected, path }
            if *expected == "namespace" && path.as_str() == format!("{lake}/absent")),
        "{error:?}"
    );
    assert_eq!(fake.request_count(), 1);
}

#[test]
fn removing_a_namespace_acts_once_and_an_absent_one_is_already_removed() {
    let fake = S3TablesFake::start();
    let tables = client(&fake);
    let lake = lake(&fake);
    fake.seed_namespace("lake", "trial");

    tables
        .remove_namespace(&lake, "trial")
        .expect("the namespace deleted");
    assert!(!fake.has_namespace("lake", "trial"));
    tables
        .remove_namespace(&lake, "trial")
        .expect("nothing left to delete");
    assert_eq!(
        fake.requests()
            .iter()
            .map(|request| (request.line(), request.status))
            .collect::<Vec<_>>(),
        [
            (format!("DELETE /namespaces/{LAKE_LABEL}/trial"), 204),
            (format!("DELETE /namespaces/{LAKE_LABEL}/trial"), 404)
        ]
    );
}

#[test]
fn removing_a_namespace_that_holds_a_table_is_the_services_conflict() {
    let fake = S3TablesFake::start();
    let tables = client(&fake);
    let lake = lake(&fake);
    fake.seed_table("lake", "trial", "events");

    let error = tables
        .remove_namespace(&lake, "trial")
        .expect_err("a namespace that is not empty");
    assert_eq!(refusal(&error), (409, "ConflictException"));
    assert!(!error.is_conflict());
    assert!(fake.has_namespace("lake", "trial"));
}
