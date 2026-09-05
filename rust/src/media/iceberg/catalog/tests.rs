//! The catalog hierarchy: catalogs of namespaces of tables, one shape each.

use std::sync::Arc;

use arrow_array::{Array, Int64Array, RecordBatch, StringArray};

use super::{Catalog, Catalogs};
use crate::IOBase;
use crate::holder::local::Folder;
use crate::media::iceberg::Transform;
use crate::{DataType, Field, IOKind};

/// Build a catalog over a scratch warehouse unique to this test and process.
fn warehouse(label: &str) -> (std::path::PathBuf, Catalog<Folder>) {
    let mut path = Folder::temporary().unwrap().path().unwrap();
    path.push(format!(
        "yggdryl-iceberg-catalog-{label}-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&path);
    let catalog = Catalog::new(Folder::new(&path).unwrap());
    (path, catalog)
}

/// The two-column taxi schema the catalog tests write, deliberately
/// unnumbered so the catalog has to number it.
fn taxi_schema() -> Field {
    DataType::from_fields([
        DataType::Int64.required_field("id"),
        DataType::Utf8.nullable_field("venue"),
    ])
    .unwrap()
    .required_field("row")
}

/// The taxi schema with `venue` marked as its own partition column.
fn marked_taxi_schema() -> Field {
    DataType::from_fields([
        DataType::Int64.required_field("id"),
        DataType::Utf8.nullable_field("venue").with_partition(true),
    ])
    .unwrap()
    .required_field("row")
}

/// Build one batch of taxis against [`taxi_schema`].
fn taxis(ids: &[i64], venues: &[Option<&str>]) -> RecordBatch {
    let schema = taxi_schema().into_arrow_schema().unwrap();
    RecordBatch::try_new(
        schema,
        vec![
            Arc::new(Int64Array::from(ids.to_vec())),
            Arc::new(StringArray::from(venues.to_vec())),
        ],
    )
    .unwrap()
}

/// Collect every row of a scan as sorted `(id, venue)` pairs.
fn collect(reader: crate::arrow::BatchReader) -> Vec<(i64, Option<String>)> {
    let mut rows = Vec::new();
    for batch in reader {
        let batch = batch.unwrap();
        let ids = batch
            .column_by_name("id")
            .unwrap()
            .as_any()
            .downcast_ref::<Int64Array>()
            .unwrap()
            .clone();
        let venues = batch
            .column_by_name("venue")
            .unwrap()
            .as_any()
            .downcast_ref::<StringArray>()
            .unwrap()
            .clone();
        for row in 0..batch.num_rows() {
            rows.push((
                ids.value(row),
                (!venues.is_null(row)).then(|| venues.value(row).to_owned()),
            ));
        }
    }
    rows.sort();
    rows
}

/// Drain a names iterator, panicking on the first failing entry.
fn names(names: super::Names) -> Vec<String> {
    names.collect::<crate::Result<Vec<_>>>().unwrap()
}

#[test]
fn constructing_a_catalog_touches_nothing() {
    let (path, catalog) = warehouse("lazy");

    // Asking questions of an empty warehouse is answered, not failed, and
    // neither the construction nor the questions bring the folder into being.
    assert_eq!(names(catalog.namespaces().iter()), Vec::<String>::new());
    assert_eq!(names(catalog.tables().iter()), Vec::<String>::new());
    assert!(!catalog.tables().contains("nyc.taxis").unwrap());
    assert!(!path.exists());
}

#[test]
fn each_catalog_role_answers_the_kind_it_plays() {
    let (_path, catalog) = warehouse("kinds");

    // Storage sees three folders; the framing is what tells them apart, so
    // each value answers for itself.
    assert_eq!(catalog.kind(), IOKind::Catalog);
    assert!(catalog.kind().is_container());

    let table = catalog.tables().create("nyc.taxis", taxi_schema()).unwrap();
    assert_eq!(IOBase::kind(&table), IOKind::Table);
    assert!(table.is_tabular());
    assert!(!table.is_atomic());

    // The plain folder route reaches the same shape by probing the location,
    // where holding the table answers it outright.
    let folder = catalog.warehouse().child_by_path("nyc/taxis").unwrap();
    assert_eq!(folder.kind(), IOKind::Directory);
    assert!(folder.is_tabular());
    assert!(!folder.is_atomic());

    // A namespace is a folder that is not a table; that it is a *namespace*
    // is what the catalog framing adds.
    let nyc = catalog.namespaces().get("nyc").unwrap();
    assert_eq!(nyc.kind(), IOKind::Namespace);
    assert!(nyc.kind().is_container());
}

#[test]
fn a_table_created_through_a_dotted_name_round_trips_its_rows() {
    let (_path, catalog) = warehouse("round-trip");

    // Two namespace levels deep, from one dotted name.
    let table = catalog
        .tables()
        .create("nyc.yellow.taxis", taxi_schema())
        .unwrap();
    assert!(table.current_snapshot().is_none());
    assert!(catalog.tables().contains("nyc.yellow.taxis").unwrap());
    assert!(!catalog.tables().contains("nyc.green.taxis").unwrap());

    let batch = taxis(&[1, 2], &[Some("XNAS"), None]);
    catalog
        .tables()
        .append_arrow_reader(
            "nyc.yellow.taxis",
            crate::arrow::batch_reader(batch.schema(), [batch]),
        )
        .unwrap();

    let table = catalog.table("nyc.yellow.taxis").unwrap();
    assert_eq!(
        collect(table.scan(None).unwrap()),
        [(1, Some("XNAS".to_owned())), (2, None)]
    );
}

#[test]
fn the_cascade_and_the_dotted_name_reach_the_same_table() {
    let (_path, catalog) = warehouse("cascade-equality");
    catalog
        .tables()
        .create("sales.eu.orders", taxi_schema())
        .unwrap();

    // The same table, three spellings: the full cascade, a dotted collection
    // name, and the catalog's dotted entry point.
    let cascaded = catalog
        .namespaces()
        .get("sales")
        .unwrap()
        .namespaces()
        .get("eu")
        .unwrap()
        .tables()
        .get("orders")
        .unwrap();
    let dotted = catalog.tables().get("sales.eu.orders").unwrap();
    let entry = catalog.table("sales.eu.orders").unwrap();

    assert_eq!(cascaded.metadata().table_uuid, dotted.metadata().table_uuid);
    assert_eq!(cascaded.metadata().table_uuid, entry.metadata().table_uuid);

    // And a dotted namespace name descends exactly as the cascade does.
    let eu = catalog.namespaces().get("sales.eu").unwrap();
    assert_eq!(eu.name(), "sales.eu");
    assert!(eu.tables().contains("orders").unwrap());
}

#[test]
fn a_table_create_makes_its_namespaces_by_writing() {
    let (path, catalog) = warehouse("ancestry");

    // Three namespace levels that do not exist: the first metadata write is
    // what brings them into being - nothing checked for them first.
    let table = catalog
        .tables()
        .create("a.b.c.orders", taxi_schema())
        .unwrap();
    assert!(table.current_snapshot().is_none());
    assert!(path.join("a/b/c/orders/metadata").is_dir());

    // The table opens through every spelling, and each ancestor namespace is
    // reachable even though none was created explicitly.
    assert!(catalog.table("a.b.c.orders").is_ok());
    assert!(catalog.namespaces().contains("a").unwrap());
    assert!(catalog.namespaces().get("a.b.c").is_ok());
}

#[test]
fn create_table_derives_the_spec_and_numbers_an_unnumbered_schema() {
    let (_path, catalog) = warehouse("marked-schema");

    let table = catalog
        .tables()
        .create("nyc.taxis", marked_taxi_schema())
        .unwrap();

    // The schema's own partition mark became the table's default spec.
    let spec = table.metadata().default_spec().unwrap();
    let names: Vec<&str> = spec
        .fields
        .iter()
        .map(|field| field.name.as_str())
        .collect();
    assert_eq!(names, ["venue"]);
    assert_eq!(spec.fields[0].transform, Transform::Identity);

    // And the unnumbered schema was numbered before anything was written.
    let schema = table.schema().unwrap();
    assert_eq!(
        schema
            .get_field_by_path("id")
            .unwrap()
            .parquet_field_id()
            .unwrap(),
        Some(1)
    );
    assert_eq!(
        schema
            .get_field_by_path("venue")
            .unwrap()
            .parquet_field_id()
            .unwrap(),
        Some(2)
    );
    assert_eq!(spec.fields[0].source_id, 2);

    // A schema that marks nothing produces the unpartitioned spec.
    let plain = catalog.tables().create("nyc.plain", taxi_schema()).unwrap();
    assert!(plain.metadata().default_spec().unwrap().is_unpartitioned());
}

#[test]
fn append_creates_on_first_write_and_appends_on_the_second() {
    let (_path, catalog) = warehouse("append-creates");

    let first = taxis(&[1], &[Some("XNAS")]);
    let table = catalog
        .tables()
        .append_arrow_reader(
            "ops.trips",
            crate::arrow::batch_reader(first.schema(), [first]),
        )
        .unwrap();

    // The inferred schema is the reader's, rooted at `row` and numbered.
    let schema = table.schema().unwrap();
    assert_eq!(schema.name(), "row");
    assert_eq!(
        schema
            .get_field_by_path("id")
            .unwrap()
            .parquet_field_id()
            .unwrap(),
        Some(1)
    );

    let second = taxis(&[2], &[None]);
    let appended = catalog
        .tables()
        .append_arrow_reader(
            "ops.trips",
            crate::arrow::batch_reader(second.schema(), [second]),
        )
        .unwrap();

    // The returned table and a fresh open through the name read the same rows.
    let rows = collect(appended.scan(None).unwrap());
    assert_eq!(rows, [(1, Some("XNAS".to_owned())), (2, None)]);
    assert_eq!(
        collect(catalog.table("ops.trips").unwrap().scan(None).unwrap()),
        rows
    );
}

#[test]
fn append_takes_the_partition_marks_that_survived_the_arrow_round_trip() {
    let (_path, catalog) = warehouse("append-marked");

    // The marks ride the Arrow fields' metadata, so a reader built from a
    // marked schema still says which columns the layout spells out.
    let arrow_schema = marked_taxi_schema().into_arrow_schema().unwrap();
    let batch = RecordBatch::try_new(
        Arc::clone(&arrow_schema),
        vec![
            Arc::new(Int64Array::from(vec![1, 2])),
            Arc::new(StringArray::from(vec![Some("XNAS"), Some("XNYS")])),
        ],
    )
    .unwrap();
    let table = catalog
        .tables()
        .append_arrow_reader(
            "nyc.marked",
            crate::arrow::batch_reader(arrow_schema, [batch]),
        )
        .unwrap();

    let spec = table.metadata().default_spec().unwrap();
    assert_eq!(spec.fields.len(), 1);
    assert_eq!(spec.fields[0].name.as_str(), "venue");
    assert_eq!(collect(table.scan(None).unwrap()).len(), 2);
}

#[test]
fn overwrite_creates_when_absent_and_replaces_when_present() {
    let (_path, catalog) = warehouse("overwrite");

    let first = taxis(&[1, 2], &[Some("XNAS"), Some("XNYS")]);
    catalog
        .tables()
        .overwrite_arrow_reader(
            "nyc.taxis",
            crate::arrow::batch_reader(first.schema(), [first]),
        )
        .unwrap();

    let second = taxis(&[9], &[None]);
    let table = catalog
        .tables()
        .overwrite_arrow_reader(
            "nyc.taxis",
            crate::arrow::batch_reader(second.schema(), [second]),
        )
        .unwrap();

    assert_eq!(collect(table.scan(None).unwrap()), [(9, None)]);
}

#[test]
fn absence_and_conflict_are_typed_at_every_level() {
    let (_path, catalog) = warehouse("typed-failures");
    catalog.tables().create("nyc.taxis", taxi_schema()).unwrap();

    // A create over an existing table is the typed conflict, from the same
    // one classification the open paths use - never from a separate probe.
    let conflict = catalog
        .tables()
        .create("nyc.taxis", taxi_schema())
        .unwrap_err();
    assert!(conflict.is_conflict(), "{conflict}");
    assert!(
        conflict.to_string().contains("nyc.taxis"),
        "the full dotted path is named: {conflict}"
    );

    // A missing table is the typed absence, naming the full dotted path.
    let absent = catalog.table("nyc.cabs").unwrap_err();
    assert!(absent.is_absent(), "{absent}");
    assert!(absent.to_string().contains("nyc.cabs"), "{absent}");

    // The same two shapes one level up.
    catalog.namespaces().create("sales").unwrap();
    let conflict = catalog.namespaces().create("sales").unwrap_err();
    assert!(conflict.is_conflict(), "{conflict}");
    let absent = catalog.namespaces().get("ops").unwrap_err();
    assert!(absent.is_absent(), "{absent}");

    // And a table's name conflicts with a namespace create, naming both.
    let message = catalog.namespaces().create("nyc.taxis").unwrap_err();
    assert!(message.is_conflict(), "{message}");
    assert!(message.to_string().contains("table"), "{message}");
}

#[test]
fn a_bad_name_segment_is_refused_by_name() {
    let (path, catalog) = warehouse("bad-names");

    let message = catalog
        .tables()
        .create("", taxi_schema())
        .unwrap_err()
        .to_string();
    assert!(message.contains("got an empty one"), "{message}");

    let message = catalog
        .tables()
        .contains("nyc..taxis")
        .unwrap_err()
        .to_string();
    assert!(message.contains("\"nyc..taxis\""), "{message}");

    let message = catalog
        .tables()
        .create("a=b", taxi_schema())
        .unwrap_err()
        .to_string();
    assert!(message.contains("\"a=b\""), "{message}");
    assert!(message.contains("partition directory"), "{message}");

    let message = catalog
        .namespaces()
        .get("nyc/taxis")
        .unwrap_err()
        .to_string();
    assert!(message.contains("'/'"), "{message}");

    // The reserved metadata name is refused at every level, because that
    // folder is where each level keeps its own document.
    let message = catalog
        .tables()
        .create("metadata", taxi_schema())
        .unwrap_err()
        .to_string();
    assert!(message.contains("metadata"), "{message}");

    // A refused name creates nothing.
    assert!(!path.exists());
}

#[test]
fn listing_sees_what_was_created_and_a_stray_folder_is_a_namespace() {
    let (path, catalog) = warehouse("listing");
    catalog.tables().create("nyc.taxis", taxi_schema()).unwrap();
    catalog.tables().create("nyc.cabs", taxi_schema()).unwrap();
    catalog.tables().create("ops.trips", taxi_schema()).unwrap();

    // A plain folder somebody else made is a namespace, never a table, and a
    // stray file is neither.
    std::fs::create_dir_all(path.join("nyc").join("scratch")).unwrap();
    std::fs::write(path.join("notes.txt"), b"not a namespace").unwrap();

    assert_eq!(names(catalog.namespaces().iter()), ["nyc", "ops"]);
    let nyc = catalog.namespaces().get("nyc").unwrap();
    assert_eq!(names(nyc.namespaces().iter()), ["scratch"]);
    assert_eq!(names(nyc.tables().iter()), ["cabs", "taxis"]);
    let ops = catalog.namespaces().get("ops").unwrap();
    assert_eq!(names(ops.tables().iter()), ["trips"]);
    assert_eq!(names(ops.namespaces().iter()), Vec::<String>::new());
    let scratch = nyc.namespaces().get("scratch").unwrap();
    assert_eq!(names(scratch.tables().iter()), Vec::<String>::new());
}

#[test]
fn the_views_are_lazy_and_answer_from_storage_at_call_time() {
    let (path, catalog) = warehouse("lazy-views");

    // Constructing every view in the chain touches nothing at all.
    let namespaces = catalog.namespaces();
    assert!(!path.exists());
    assert_eq!(names(namespaces.iter()), Vec::<String>::new());
    assert_eq!(namespaces.len().unwrap(), 0);
    assert!(namespaces.is_empty().unwrap());
    assert!(!namespaces.contains("nyc").unwrap());
    assert!(!path.exists());

    // A view constructed before a write observes the write, because every
    // answer comes from storage when the question is asked.
    catalog.tables().create("nyc.taxis", taxi_schema()).unwrap();
    assert_eq!(names(namespaces.iter()), ["nyc"]);
    assert!(namespaces.contains("nyc").unwrap());
    let nyc = namespaces.get("nyc").unwrap();
    assert_eq!(names(nyc.tables().iter()), ["taxis"]);

    // Two views over the same catalog observe each other's writes.
    let tables = nyc.tables();
    let second = catalog.namespaces().get("nyc").unwrap().tables();
    second.create("cabs", taxi_schema()).unwrap();
    assert_eq!(names(tables.iter()), ["cabs", "taxis"]);
    assert_eq!(tables.len().unwrap(), 2);
    assert!(tables.contains("cabs").unwrap());
}

#[test]
fn access_chains_through_the_views_and_a_missing_name_is_named() {
    let (_path, catalog) = warehouse("chained-views");
    catalog
        .tables()
        .create("nyc.yellow.taxis", taxi_schema())
        .unwrap();

    // The cascade: a nested namespace is reached through its parent's view.
    let nyc = catalog.namespaces().get("nyc").unwrap();
    let yellow = nyc.namespaces().get("yellow").unwrap();
    assert_eq!(yellow.name(), "nyc.yellow");
    let table = yellow.tables().get("taxis").unwrap();
    assert!(table.current_snapshot().is_none());

    // A missing table is a typed error naming the full dotted path.
    let message = yellow.tables().get("cabs").unwrap_err().to_string();
    assert!(message.contains("nyc.yellow.cabs"), "{message}");

    // A missing namespace is a typed error naming the namespace, and a
    // table's name is not a namespace.
    let message = catalog.namespaces().get("ops").unwrap_err().to_string();
    assert!(message.contains("\"ops\""), "{message}");
    assert!(!nyc.namespaces().contains("taxis").unwrap());
    let message = nyc
        .namespaces()
        .get("yellow.taxis")
        .unwrap_err()
        .to_string();
    assert!(message.contains("table"), "{message}");
}

#[test]
fn an_empty_namespace_is_durable_and_survives_a_reopen() {
    let (path, catalog) = warehouse("durable-namespace");

    // The namespace document is what makes an empty one durable - no marker
    // trick, no zero-byte truncation.
    let sales = catalog.namespaces().create("sales").unwrap();
    assert_eq!(sales.name(), "sales");
    assert!(path.join("sales/metadata/namespace.json").is_file());

    // A second catalog over the same folder sees it, holding no tables.
    let reopened = Catalog::new(Folder::new(&path).unwrap());
    assert!(reopened.namespaces().contains("sales").unwrap());
    let sales = reopened.namespaces().get("sales").unwrap();
    assert!(sales.tables().is_empty().unwrap());
    assert_eq!(names(reopened.namespaces().iter()), ["sales"]);

    // Its own metadata folder is infrastructure, never a child namespace.
    assert_eq!(names(sales.namespaces().iter()), Vec::<String>::new());
}

#[test]
fn properties_round_trip_at_all_three_levels() {
    let mut root = Folder::temporary().unwrap().path().unwrap();
    root.push(format!("yggdryl-iceberg-catalogs-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);

    // The catalogs level: a folder of warehouses.
    let catalogs = Catalogs::new(Folder::new(&root).unwrap());
    let lake = catalogs.create("lake").unwrap();
    assert!(catalogs.contains("lake").unwrap());
    assert_eq!(
        catalogs.iter().collect::<crate::Result<Vec<_>>>().unwrap(),
        ["lake"]
    );

    // Catalog properties live in metadata/catalog.json under the warehouse.
    assert!(lake.properties().unwrap().is_empty());
    lake.update_properties([("owner".to_owned(), "ops".to_owned())], [])
        .unwrap();
    assert_eq!(
        lake.properties().unwrap().get("owner").map(String::from),
        Some("ops".to_owned())
    );

    // Namespace properties live in metadata/namespace.json under the folder.
    let sales = lake.namespaces().create("sales").unwrap();
    assert!(sales.properties().unwrap().is_empty());
    sales
        .update_properties(
            [
                ("region".to_owned(), "eu".to_owned()),
                ("tier".to_owned(), "gold".to_owned()),
            ],
            [],
        )
        .unwrap();
    sales.update_properties([], ["tier".to_owned()]).unwrap();
    let properties = sales.properties().unwrap();
    assert_eq!(
        properties.get("region").map(String::from),
        Some("eu".to_owned())
    );
    assert!(properties.get("tier").is_none());

    // The reserved prefix is refused by name, and the refusal changes nothing.
    let refused = sales
        .update_properties([("iceberg:spec".to_owned(), "x".to_owned())], [])
        .unwrap_err()
        .to_string();
    assert!(refused.contains("iceberg:"), "{refused}");
    assert_eq!(sales.properties().unwrap().len(), 1);

    // Table properties already ride TableMetadata - assert the level exists.
    let table = lake.tables().create("sales.orders", taxi_schema()).unwrap();
    assert!(table.metadata().properties.is_empty());

    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn an_absent_properties_document_answers_empty_at_every_level() {
    let (path, catalog) = warehouse("absent-properties");

    // A fresh catalog over an empty warehouse: no folder, no document, and
    // still an answer - absent means empty, never a missing-file failure.
    assert!(catalog.properties().unwrap().is_empty());
    assert!(!path.exists());

    // A namespace a table write brought into being carries no document of its
    // own, and it answers the same way.
    catalog.tables().create("nyc.taxis", taxi_schema()).unwrap();
    assert!(!path.join("nyc/metadata/namespace.json").exists());
    let nyc = catalog.namespaces().get("nyc").unwrap();
    assert!(nyc.properties().unwrap().is_empty());
}

#[test]
fn a_malformed_properties_document_is_refused_naming_what_was_found() {
    let (path, catalog) = warehouse("malformed-properties");
    catalog.namespaces().create("sales").unwrap();
    let document = path.join("sales/metadata/namespace.json");
    let read = || {
        catalog
            .namespaces()
            .get("sales")
            .unwrap()
            .properties()
            .unwrap_err()
            .to_string()
    };

    // A document without the one expected key - here a JSON array.
    std::fs::write(&document, "[1, 2]").unwrap();
    let error = read();
    assert!(
        error.contains("expected a {\"properties\": ...} document at"),
        "{error}"
    );
    assert!(error.contains("got one without the key"), "{error}");
    assert!(error.contains("namespace.json"), "{error}");

    // The key is there, but it holds a sequence rather than a mapping.
    std::fs::write(&document, "{\"properties\": [1, 2]}").unwrap();
    let error = read();
    assert!(
        error.contains("expected \"properties\" to hold a mapping at"),
        "{error}"
    );
    assert!(error.contains("namespace.json"), "{error}");

    // The mapping is there, but a value is not a string.
    std::fs::write(&document, "{\"properties\": {\"threshold\": 10}}").unwrap();
    let error = read();
    assert!(
        error.contains("expected string property pairs at"),
        "{error}"
    );
    assert!(error.contains("threshold"), "{error}");
}

#[test]
fn the_reserved_prefix_is_refused_at_the_catalog_level_too() {
    let (_path, catalog) = warehouse("reserved-catalog");

    // The same refusal the namespace level gives, one level up, and the
    // refusal changes nothing: the document stays absent.
    let refused = catalog
        .update_properties([("iceberg:spec".to_owned(), "x".to_owned())], [])
        .unwrap_err()
        .to_string();
    assert!(refused.contains("reserved"), "{refused}");
    assert!(refused.contains("iceberg:"), "{refused}");
    assert!(catalog.properties().unwrap().is_empty());
}

#[test]
fn a_catalog_names_itself_after_its_warehouse_folder() {
    let (path, catalog) = warehouse("named");

    // The name is the warehouse folder's own, the identity `Catalogs::iter`
    // lists it under - answered from the handle, no I/O.
    assert_eq!(
        catalog.name(),
        path.file_name().and_then(|name| name.to_str())
    );
    assert!(!path.exists());

    // An in-memory warehouse names itself after its identity URL the same
    // way, so every warehouse has the answer its handle can give.
    let memory = Catalog::new(crate::holder::Buffer::new());
    assert_eq!(
        memory.name(),
        memory.warehouse().url().and_then(crate::Url::file_name)
    );
}

#[test]
fn two_creators_of_one_table_converge_or_one_gets_the_typed_conflict() {
    let (path, _catalog) = warehouse("racing-creates");

    // Two threads, two catalogs, one name. Storage has no compare-and-swap,
    // so the contract is: both converge on the same table, or one of them
    // gets the typed conflict - never corruption, never a silent third state.
    let barrier = std::sync::Barrier::new(2);
    fn make(path: &std::path::Path, barrier: &std::sync::Barrier) -> crate::Result<()> {
        let catalog = Catalog::new(Folder::new(path).unwrap());
        barrier.wait();
        catalog
            .tables()
            .create("race.orders", taxi_schema())
            .map(|_| ())
    }
    let outcomes: Vec<crate::Result<()>> = std::thread::scope(|scope| {
        let left = scope.spawn(|| make(&path, &barrier));
        let right = scope.spawn(|| make(&path, &barrier));
        vec![left.join().unwrap(), right.join().unwrap()]
    });

    let successes = outcomes.iter().filter(|outcome| outcome.is_ok()).count();
    assert!(successes >= 1, "{outcomes:?}");
    for outcome in &outcomes {
        if let Err(error) = outcome {
            // Storage has no compare-and-swap and the local backend has no
            // atomic publish, so the loser either classifies after the winner
            // finished - the typed conflict - or collides with the winner's
            // in-flight document. Unix can expose that as a partial codec
            // read, while Windows can refuse the losing resize because the
            // winner still owns a mapped section. Both are the race being
            // reported; what the contract forbids is silence and corruption,
            // and the reopen below is the corruption check.
            assert!(
                error.is_conflict()
                    || matches!(error, crate::Error::Codec { .. } | crate::Error::Io(_)),
                "{error}"
            );
        }
    }

    // Whoever won, the table is whole and opens.
    let catalog = Catalog::new(Folder::new(&path).unwrap());
    let table = catalog.table("race.orders").unwrap();
    assert!(table.current_snapshot().is_none());
}

#[test]
fn table_writes_through_the_views_create_on_first_write() {
    let (_path, catalog) = warehouse("view-writes");

    let sales = catalog.namespaces().open_or_create("sales").unwrap();
    let batch = taxis(&[1, 2], &[Some("XNAS"), None]);
    let table = sales
        .tables()
        .append_arrow_reader(
            "orders",
            crate::arrow::batch_reader(batch.schema(), [batch]),
        )
        .unwrap();
    assert_eq!(collect(table.scan(None).unwrap()).len(), 2);

    // The overwrite convenience replaces through the same view.
    let batch = taxis(&[9], &[None]);
    let table = sales
        .tables()
        .overwrite_arrow_reader(
            "orders",
            crate::arrow::batch_reader(batch.schema(), [batch]),
        )
        .unwrap();
    assert_eq!(collect(table.scan(None).unwrap()), [(9, None)]);

    // The dotted entry point is the same implementation, so both spellings
    // observe the same table.
    assert_eq!(
        collect(catalog.table("sales.orders").unwrap().scan(None).unwrap()),
        [(9, None)]
    );
}

/// The number of backend calls an operation makes is a behavior, not an
/// implementation detail - the expression module's selector cost tests set
/// that precedent, and the existence audit's whole point is the round trips
/// that are no longer spent asking questions whose answers were stale.
mod call_counts {
    use std::any::Any;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::{Catalog, taxi_schema};
    use crate::Result;
    use crate::holder::fs::{
        ByteReader, ByteWriter, FileInfo, FileInfos, FileSelector, FileSystem, Folder,
        MemoryFileSystem, OutputMetadata, RandomAccessReader,
    };

    /// A memory filesystem that counts every vtable call reaching it.
    #[derive(Debug, Default)]
    struct Counting {
        inner: MemoryFileSystem,
        calls: AtomicUsize,
    }

    impl Counting {
        fn calls(&self) -> usize {
            self.calls.load(Ordering::Relaxed)
        }

        fn count(&self) {
            self.calls.fetch_add(1, Ordering::Relaxed);
        }
    }

    impl FileSystem for Counting {
        fn type_name(&self) -> &str {
            self.inner.type_name()
        }

        fn equals(&self, other: &dyn FileSystem) -> bool {
            self.count();
            other
                .as_any()
                .downcast_ref::<Self>()
                .is_some_and(|other| std::ptr::eq(self, other))
        }

        fn normalize_path(&self, path: &str) -> Result<String> {
            self.count();
            self.inner.normalize_path(path)
        }

        fn file_info(&self, path: &str) -> Result<FileInfo> {
            self.count();
            self.inner.file_info(path)
        }

        fn list(&self, selector: &FileSelector) -> FileInfos {
            self.count();
            self.inner.list(selector)
        }

        fn create_dir(&self, path: &str, recursive: bool) -> Result<()> {
            self.count();
            self.inner.create_dir(path, recursive)
        }

        fn delete_dir(&self, path: &str) -> Result<()> {
            self.count();
            self.inner.delete_dir(path)
        }

        fn delete_dir_contents(&self, path: &str, missing_dir_ok: bool) -> Result<()> {
            self.count();
            self.inner.delete_dir_contents(path, missing_dir_ok)
        }

        fn delete_root_dir_contents(&self) -> Result<()> {
            self.count();
            self.inner.delete_root_dir_contents()
        }

        fn delete_file(&self, path: &str) -> Result<()> {
            self.count();
            self.inner.delete_file(path)
        }

        fn copy_file(&self, source: &str, target: &str) -> Result<()> {
            self.count();
            self.inner.copy_file(source, target)
        }

        fn move_file(&self, source: &str, target: &str) -> Result<()> {
            self.count();
            self.inner.move_file(source, target)
        }

        fn open_input_file(&self, path: &str) -> Result<Box<dyn RandomAccessReader>> {
            self.count();
            self.inner.open_input_file(path)
        }

        fn open_input_stream(&self, path: &str) -> Result<Box<dyn ByteReader>> {
            self.count();
            self.inner.open_input_stream(path)
        }

        fn open_output_stream(
            &self,
            path: &str,
            metadata: Option<&OutputMetadata>,
        ) -> Result<Box<dyn ByteWriter>> {
            self.count();
            self.inner.open_output_stream(path, metadata)
        }

        fn open_append_stream(
            &self,
            path: &str,
            metadata: Option<&OutputMetadata>,
        ) -> Result<Box<dyn ByteWriter>> {
            self.count();
            self.inner.open_append_stream(path, metadata)
        }

        fn as_any(&self) -> &dyn Any {
            self
        }
    }

    /// A catalog over a counting warehouse, with the counter beside it.
    fn counted() -> (Arc<Counting>, Catalog<Folder>) {
        let filesystem = Arc::new(Counting::default());
        let warehouse = Folder::from_path(
            Arc::clone(&filesystem) as Arc<dyn FileSystem>,
            "warehouse",
            None,
        )
        .expect("a valid location");
        (filesystem, Catalog::new(warehouse))
    }

    /// One measured run: the calls `operation` makes on a fresh counter.
    fn cost(filesystem: &Counting, operation: impl FnOnce()) -> usize {
        let before = filesystem.calls();
        operation();
        filesystem.calls() - before
    }

    #[test]
    fn a_get_of_an_existing_table_is_one_resolution_and_one_locate() {
        let (filesystem, catalog) = counted();
        catalog
            .tables()
            .create("sales.orders", taxi_schema())
            .unwrap();

        // Raw child resolution is local. The retained metadata streams avoid
        // duplicate classification and open calls.
        let calls = cost(&filesystem, || {
            catalog.tables().get("sales.orders").unwrap();
        });
        assert_eq!(calls, 4, "get on an existing table");
    }

    #[test]
    fn a_get_of_a_missing_table_stops_at_the_presence_answer() {
        let (filesystem, catalog) = counted();
        catalog
            .tables()
            .create("sales.orders", taxi_schema())
            .unwrap();

        // The child is derived from the retained raw path without a backend
        // call. One file-info answer says it is absent, so no locate or
        // metadata-directory listing runs.
        let calls = cost(&filesystem, || {
            catalog.tables().get("sales.nothing").unwrap_err();
        });
        assert_eq!(calls, 1, "get on a missing table");
    }

    #[test]
    fn a_create_into_a_missing_ancestry_never_walks_the_ancestry() {
        let (filesystem, catalog) = counted();

        // One presence answer, then the create's direct stream operations.
        // The first output open reports a missing parent, one recursive
        // create repairs it, and the output open is retried exactly once.
        // Raw child resolution never walks ancestor namespaces.
        let calls = cost(&filesystem, || {
            catalog
                .tables()
                .create("a.b.c.orders", taxi_schema())
                .unwrap();
        });
        assert_eq!(calls, 14, "create into three missing namespace levels");

        // And the table it made opens.
        catalog.tables().get("a.b.c.orders").unwrap();
    }

    #[test]
    fn open_or_create_costs_the_same_one_classification_on_both_branches() {
        let (filesystem, catalog) = counted();

        // The absent branch: one classification, then the create's writes -
        // exactly what `create` costs, because it is the same attempt.
        let absent = cost(&filesystem, || {
            catalog
                .tables()
                .open_or_create("sales.orders", taxi_schema())
                .unwrap();
        });
        assert_eq!(absent, 14, "open_or_create when absent");

        // The present branch: one classification, whose locate already opened
        // the table - exactly what `get` costs, because it is the same
        // attempt.
        let present = cost(&filesystem, || {
            catalog
                .tables()
                .open_or_create("sales.orders", taxi_schema())
                .unwrap();
        });
        assert_eq!(present, 4, "open_or_create when present");
    }
}
