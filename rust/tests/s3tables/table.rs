//! `rust/src/s3tables/table.rs`: the table verbs and the values they
//! answer - a creation with and without a schema, a description, the
//! metadata location and the commit that moves it under a version token, a
//! rename, a deletion.

use serde_json::json;
use yggdryl::{DataType, DateTime64, Error, StructType, Timezone, Url};

use crate::fake::S3TablesFake;
use crate::mod_::{LAKE_LABEL, arn, client, lake, refusal, schema};

/// A client, the table bucket `lake`, and the namespace `trial` in it.
fn trial(fake: &S3TablesFake) -> (yggdryl::s3tables::S3Tables, yggdryl::Arn) {
    let lake = lake(fake);
    fake.seed_namespace("lake", "trial");
    (client(fake), lake)
}

/// A metadata file's location under `warehouse`.
fn metadata_file(warehouse: &str, name: &str) -> Url {
    Url::from_str(&format!("{warehouse}/metadata/{name}.metadata.json")).expect("a location")
}

#[test]
fn a_table_is_created_with_the_iceberg_schema_its_field_spells() {
    let fake = S3TablesFake::start();
    let (tables, lake) = trial(&fake);

    let created = tables
        .create_table(&lake, "trial", "events", Some(&schema()))
        .expect("a table");
    let held = fake.table("lake", "trial", "events").expect("the table");
    assert_eq!(created.arn().to_string(), held.arn);
    assert_eq!(created.arn().table(), held.arn.rsplit('/').next());
    assert_eq!(created.version_token(), held.version_token);

    // The body is the model's: the name, the one format, and the schema as
    // the crate's own Iceberg writer renders it - the columns numbered,
    // since the field carried no id, and typed as Iceberg spells them.
    let recorded = fake.requests();
    assert_eq!(recorded.len(), 1);
    assert_eq!(
        recorded[0].line(),
        format!("PUT /tables/{LAKE_LABEL}/trial")
    );
    assert_eq!(
        recorded[0].json(),
        json!({
            "name": "events",
            "format": "ICEBERG",
            "metadata": {"iceberg": {"schemaV2": {
                "type": "struct",
                "fields": [
                    {"id": 1, "name": "id", "required": true, "type": "long"},
                    {"id": 2, "name": "symbol", "required": false, "type": "string"},
                    {"id": 3, "name": "ts", "required": false, "type": "timestamptz"},
                ],
            }}},
        })
    );
    assert_eq!(
        held.schema,
        Some(recorded[0].json()["metadata"]["iceberg"]["schemaV2"].clone())
    );
    // The caller's field is read, never numbered in place.
    assert_eq!(
        schema().fields()[0].parquet_field_id().expect("an id"),
        None
    );
}

#[test]
fn a_schema_that_carries_ids_keeps_them_and_the_rest_are_numbered_above() {
    let fake = S3TablesFake::start();
    let (tables, lake) = trial(&fake);

    let mut symbol = DataType::utf8().nullable_field("symbol");
    symbol.set_parquet_field_id(7);
    let row = DataType::from(
        StructType::from_fields([
            DataType::Int64.required_field("id"),
            symbol,
            DataType::Float64.nullable_field("price"),
        ])
        .expect("three columns"),
    )
    .required_field("row");

    tables
        .create_table(&lake, "trial", "quotes", Some(&row))
        .expect("a table");
    let fields = fake
        .table("lake", "trial", "quotes")
        .expect("the table")
        .schema
        .expect("a schema")["fields"]
        .clone();
    assert_eq!(
        fields,
        json!([
            {"id": 8, "name": "id", "required": true, "type": "long"},
            {"id": 7, "name": "symbol", "required": false, "type": "string"},
            {"id": 9, "name": "price", "required": false, "type": "double"},
        ])
    );
}

#[test]
fn a_schema_that_declares_a_partition_and_an_order_creates_a_table_laid_out_so() {
    let fake = S3TablesFake::start();
    let (tables, lake) = trial(&fake);

    let mut row = DataType::from(
        StructType::from_fields([
            DataType::Int64.required_field("id"),
            DataType::utf8().nullable_field("venue"),
            DataType::Float64.nullable_field("price"),
        ])
        .expect("three columns"),
    )
    .required_field("row")
    .with_partition_fields(&["venue"])
    .expect("a partition");
    row.as_sort_mut()
        .set_by_texts(["price desc nulls first"])
        .expect("an order");

    tables
        .create_table(&lake, "trial", "quotes", Some(&row))
        .expect("a table");
    // The layout the schema declares is the table's, named by the ids the
    // schema beside it was numbered with - never dropped on the way.
    let held = fake.table("lake", "trial", "quotes").expect("the table");
    assert_eq!(
        held.partition_spec,
        Some(json!({
            "spec-id": 0,
            "fields": [
                {"name": "venue", "transform": "identity", "source-id": 2, "field-id": 1000},
            ],
        }))
    );
    assert_eq!(
        held.write_order,
        Some(json!({
            "order-id": 1,
            "fields": [
                {"source-id": 3, "transform": "identity", "direction": "desc", "null-order": "nulls-first"},
            ],
        }))
    );
    assert_eq!(
        held.schema.expect("a schema")["fields"][1],
        json!({"id": 2, "name": "venue", "required": false, "type": "string"})
    );

    // A schema declaring neither states neither.
    tables
        .create_table(&lake, "trial", "events", Some(&schema()))
        .expect("a table");
    let plain = fake.table("lake", "trial", "events").expect("the table");
    assert_eq!((plain.partition_spec, plain.write_order), (None, None));
}

#[test]
fn a_table_created_without_a_schema_has_no_metadata_location_yet() {
    let fake = S3TablesFake::start();
    let (tables, lake) = trial(&fake);

    tables
        .create_table(&lake, "trial", "events", None)
        .expect("a table");
    assert_eq!(
        fake.requests()[0].json(),
        json!({"name": "events", "format": "ICEBERG"})
    );
    let table = tables.get_table(&lake, "trial", "events").expect("a table");
    assert_eq!(table.metadata_location(), None);
    let location = tables
        .get_table_metadata_location(&lake, "trial", "events")
        .expect("a location");
    assert_eq!(location.metadata_location(), None);
    assert_eq!(location.warehouse_location(), table.warehouse_location());
}

#[test]
fn a_table_or_a_schema_the_model_refuses_costs_no_request() {
    let fake = S3TablesFake::start();
    let (tables, lake) = trial(&fake);

    let long = "t".repeat(256);
    for (name, position) in [
        ("", 0),
        ("Events", 0),
        ("with-hyphen", 4),
        (long.as_str(), 255),
    ] {
        let refused = |error: Error| match error {
            Error::Parse {
                target,
                position: at,
                reason,
            } => {
                assert_eq!(target, "table name");
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
                .create_table(&lake, "trial", name, None)
                .expect_err("a refused name"),
        );
        refused(
            tables
                .get_table(&lake, "trial", name)
                .expect_err("a refused name"),
        );
        refused(
            tables
                .get_table_metadata_location(&lake, "trial", name)
                .expect_err("a refused name"),
        );
        refused(
            tables
                .remove_table(&lake, "trial", name, None)
                .expect_err("a refused name"),
        );
        refused(
            tables
                .rename_table(&lake, "trial", "events", None, Some(name), None)
                .expect_err("a refused name"),
        );
    }
    // A namespace is checked before the table below it.
    assert!(matches!(
        tables.get_table(&lake, "Trial", "events"),
        Err(Error::Parse { target, .. }) if target == "namespace name"
    ));
    assert!(matches!(
        tables.rename_table(&lake, "trial", "events", Some("New-Desk"), None, None),
        Err(Error::Parse { target, .. }) if target == "namespace name"
    ));

    // A schema that is not a row - a non-null struct - is the Iceberg
    // writer's refusal, before anything is sent.
    let column = DataType::Int64.required_field("id");
    tables
        .create_table(&lake, "trial", "events", Some(&column))
        .expect_err("a column is not a schema");
    assert_eq!(fake.request_count(), 0);
}

#[test]
fn a_table_that_is_already_there_is_a_conflict_and_one_under_nothing_the_services_not_found() {
    let fake = S3TablesFake::start();
    let (tables, lake) = trial(&fake);
    fake.seed_table("lake", "trial", "events");

    let error = tables
        .create_table(&lake, "trial", "events", None)
        .expect_err("a table of that name");
    assert!(error.is_conflict());
    assert!(
        matches!(&error, Error::Conflict { expected, path, .. }
            if *expected == "table" && path.as_str() == format!("{lake}/trial/events")),
        "{error:?}"
    );

    let error = tables
        .create_table(&lake, "absent", "events", None)
        .expect_err("no such namespace");
    assert_eq!(refusal(&error), (404, "NotFoundException"));
    assert!(!error.is_absent());
}

#[test]
fn a_table_is_described_as_the_service_states_it() {
    let fake = S3TablesFake::start();
    let (tables, lake) = trial(&fake);
    let held = fake.seed_table("lake", "trial", "events");

    let table = tables.get_table(&lake, "trial", "events").expect("a table");
    assert_eq!(table.arn(), &arn(&held.arn));
    assert_eq!(table.name(), "events");
    assert_eq!(table.namespace(), "trial");
    assert_eq!(table.version_token(), held.version_token);
    assert_eq!(table.format(), "ICEBERG");
    // The warehouse is an `s3:` location, which the S3 backend reads and
    // writes; the metadata file lies under it.
    assert_eq!(
        table.warehouse_location().to_string(),
        held.warehouse_location
    );
    assert_eq!(table.warehouse_location().scheme().as_str(), "s3");
    assert!(
        table
            .warehouse_location()
            .bucket()
            .is_some_and(|bucket| bucket.ends_with("--table-s3"))
    );
    assert_eq!(
        table.metadata_location().map(ToString::to_string),
        held.metadata_location
    );
    // The fake stamps one tick per change: the bucket, the namespace, each
    // seeded again as the next level is, and then the table.
    let stamped =
        DateTime64::from_text("2026-01-01T00:00:06.000006Z", Timezone::UTC).expect("an instant");
    assert_eq!(table.created_at(), stamped);
    assert_eq!(table.modified_at(), stamped);
    assert_eq!(table.clone(), table);

    // The one operation that addresses a table by query spells the ARN
    // there, encoded once like any query value.
    let recorded = fake.requests();
    assert_eq!(
        recorded[0].target,
        format!("/get-table?name=events&namespace=trial&tableBucketARN={LAKE_LABEL}")
    );
    assert_eq!(
        recorded[0].query("tableBucketARN"),
        Some(lake.to_string().as_str())
    );
}

#[test]
fn a_table_that_is_not_there_is_absent() {
    let fake = S3TablesFake::start();
    let (tables, lake) = trial(&fake);

    for error in [
        tables
            .get_table(&lake, "trial", "absent")
            .expect_err("no such table"),
        tables
            .get_table_metadata_location(&lake, "trial", "absent")
            .expect_err("no such table"),
        tables
            .get_table(&lake, "absent", "events")
            .expect_err("no such namespace"),
    ] {
        assert!(error.is_absent(), "{error:?}");
        assert!(
            matches!(&error, Error::Absent { expected, path }
                if *expected == "table" && path.starts_with(lake.to_string().as_str())),
            "{error:?}"
        );
    }
    assert_eq!(fake.request_count(), 3);
}

#[test]
fn a_commit_moves_the_metadata_location_under_the_version_token() {
    let fake = S3TablesFake::start();
    let (tables, lake) = trial(&fake);
    let held = fake.seed_table("lake", "trial", "events");

    let location = tables
        .get_table_metadata_location(&lake, "trial", "events")
        .expect("a location");
    assert_eq!(location.version_token(), held.version_token);
    assert_eq!(
        location.warehouse_location().to_string(),
        held.warehouse_location
    );
    assert_eq!(location.clone(), location);

    let next = metadata_file(&held.warehouse_location, "00001-next");
    fake.clear_requests();
    let committed = tables
        .update_table_metadata_location(&lake, "trial", "events", location.version_token(), &next)
        .expect("a commit");
    assert_eq!(committed.arn().to_string(), held.arn);
    assert_ne!(committed.version_token(), held.version_token);
    assert_eq!(committed.clone(), committed);

    let recorded = fake.requests();
    assert_eq!(
        recorded.len(),
        1,
        "a commit is one request, read back by none"
    );
    assert_eq!(
        recorded[0].json(),
        json!({"versionToken": held.version_token, "metadataLocation": next.to_string()})
    );
    let now = fake.table("lake", "trial", "events").expect("the table");
    assert_eq!(now.metadata_location, Some(next.to_string()));
    assert_eq!(now.version_token, committed.version_token());
}

#[test]
fn a_commit_under_a_stale_token_is_the_services_conflict_and_moves_nothing() {
    let fake = S3TablesFake::start();
    let (tables, lake) = trial(&fake);
    let held = fake.seed_table("lake", "trial", "events");

    let won = metadata_file(&held.warehouse_location, "00001-won");
    let lost = metadata_file(&held.warehouse_location, "00001-lost");
    tables
        .update_table_metadata_location(&lake, "trial", "events", &held.version_token, &won)
        .expect("the commit that wins");

    // The loser read the same token; the table has moved past it.
    let error = tables
        .update_table_metadata_location(&lake, "trial", "events", &held.version_token, &lost)
        .expect_err("a stale token");
    assert_eq!(refusal(&error), (409, "ConflictException"));
    assert!(
        matches!(&error, Error::Remote { operation, path, .. }
            if *operation == "UpdateTableMetadataLocation"
                && path.as_str() == format!("{lake}/trial/events")),
        "{error:?}"
    );
    assert_eq!(
        fake.table("lake", "trial", "events")
            .expect("the table")
            .metadata_location,
        Some(won.to_string())
    );

    // A commit to a table that is not there is the service's own 404.
    let error = tables
        .update_table_metadata_location(&lake, "trial", "absent", &held.version_token, &lost)
        .expect_err("no such table");
    assert_eq!(refusal(&error), (404, "NotFoundException"));
}

#[test]
fn a_rename_states_only_what_it_changes_and_moves_the_token() {
    let fake = S3TablesFake::start();
    let (tables, lake) = trial(&fake);
    fake.seed_namespace("lake", "desk");
    let held = fake.seed_table("lake", "trial", "events");

    // A new name alone.
    tables
        .rename_table(&lake, "trial", "events", None, Some("fills"), None)
        .expect("a rename");
    assert_eq!(fake.requests()[0].json(), json!({"newName": "fills"}));
    let renamed = fake.table("lake", "trial", "fills").expect("the table");
    assert_eq!(renamed.arn, held.arn, "the table keeps its identity");
    assert_ne!(renamed.version_token, held.version_token);

    // A stale token keeps the table where it is.
    let error = tables
        .rename_table(
            &lake,
            "trial",
            "fills",
            Some("desk"),
            None,
            Some(&held.version_token),
        )
        .expect_err("a stale token");
    assert_eq!(refusal(&error), (409, "ConflictException"));
    assert!(fake.table("lake", "trial", "fills").is_some());

    // Another namespace, under the token the table has now.
    fake.clear_requests();
    tables
        .rename_table(
            &lake,
            "trial",
            "fills",
            Some("desk"),
            Some("orders"),
            Some(&renamed.version_token),
        )
        .expect("a move");
    assert_eq!(
        fake.requests()[0].json(),
        json!({
            "newNamespaceName": "desk",
            "newName": "orders",
            "versionToken": renamed.version_token,
        })
    );
    assert!(fake.table("lake", "trial", "fills").is_none());
    assert!(fake.table("lake", "desk", "orders").is_some());

    // A target namespace that is not there is the service's own 404.
    let error = tables
        .rename_table(&lake, "desk", "orders", Some("absent"), None, None)
        .expect_err("no such namespace");
    assert_eq!(refusal(&error), (404, "NotFoundException"));
}

#[test]
fn a_rename_that_changes_nothing_and_an_empty_token_cost_no_request() {
    let fake = S3TablesFake::start();
    let (tables, lake) = trial(&fake);
    let held = fake.seed_table("lake", "trial", "events");
    let next = metadata_file(&held.warehouse_location, "00001-next");

    // Neither a namespace nor a name, or the ones the table has: nothing to
    // change, so nothing is sent.
    for (new_namespace, new_name, said) in [
        (None, None, "neither"),
        (Some("trial"), None, "the ones it has"),
        (None, Some("events"), "the ones it has"),
        (Some("trial"), Some("events"), "the ones it has"),
    ] {
        let error = tables
            .rename_table(&lake, "trial", "events", new_namespace, new_name, None)
            .expect_err("a rename to nothing");
        assert!(
            matches!(&error, Error::Io(io) if io.kind() == std::io::ErrorKind::InvalidInput),
            "{error:?}"
        );
        assert!(error.to_string().ends_with(said), "{error}");
    }

    // A version token is at least one character, wherever it goes.
    let empty = |error: Error| {
        assert!(
            matches!(&error, Error::Parse { target, .. } if *target == "version token"),
            "{error:?}"
        );
    };
    empty(
        tables
            .rename_table(&lake, "trial", "events", None, Some("fills"), Some(""))
            .expect_err("an empty token"),
    );
    empty(
        tables
            .remove_table(&lake, "trial", "events", Some(""))
            .expect_err("an empty token"),
    );
    empty(
        tables
            .update_table_metadata_location(&lake, "trial", "events", "", &next)
            .expect_err("an empty token"),
    );
    assert_eq!(fake.request_count(), 0);
    assert!(fake.table("lake", "trial", "events").is_some());
}

#[test]
fn removing_a_table_acts_once_under_its_token_and_an_absent_one_is_already_removed() {
    let fake = S3TablesFake::start();
    let (tables, lake) = trial(&fake);
    let held = fake.seed_table("lake", "trial", "events");
    let next = metadata_file(&held.warehouse_location, "00001-next");
    let committed = tables
        .update_table_metadata_location(&lake, "trial", "events", &held.version_token, &next)
        .expect("a commit");

    // A table changed since the token was read is kept.
    let error = tables
        .remove_table(&lake, "trial", "events", Some(&held.version_token))
        .expect_err("a stale token");
    assert_eq!(refusal(&error), (409, "ConflictException"));
    assert!(fake.table("lake", "trial", "events").is_some());

    fake.clear_requests();
    tables
        .remove_table(&lake, "trial", "events", Some(committed.version_token()))
        .expect("the table deleted");
    assert!(fake.table("lake", "trial", "events").is_none());
    tables
        .remove_table(&lake, "trial", "events", None)
        .expect("nothing left to delete");
    assert_eq!(
        fake.requests()
            .iter()
            .map(|request| (request.line(), request.status))
            .collect::<Vec<_>>(),
        [
            (
                format!(
                    "DELETE /tables/{LAKE_LABEL}/trial/events?versionToken={}",
                    committed.version_token()
                ),
                204
            ),
            (format!("DELETE /tables/{LAKE_LABEL}/trial/events"), 404)
        ]
    );
}
