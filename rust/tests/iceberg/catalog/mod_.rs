//! `rust/src/iceberg/catalog/mod.rs`: an Iceberg warehouse folder as a catalog
//! of namespaces of tables, on the warehouse's one abstraction.
//!
//! Every level is one owned object - `IcebergCatalog`, `IcebergNamespace`,
//! `IcebergTable<Handle>` - held by the generic enums and walked through the
//! generic views, so the suite reaches the crate through `yggdryl::iceberg`
//! and the crate root and nothing else.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use arrow_array::{Array, Int64Array, RecordBatch, StringArray};

use yggdryl::holder::Holder;
use yggdryl::iceberg::{IcebergCatalog, IcebergTable, Transform};
use yggdryl::local::LocalFolder;
use yggdryl::{
    Catalog, CatalogValue, DataType, Field, Handle, IOBase, IOKind, IOMedia, Names, Namespace,
    NamespaceValue, ObjectValue, Properties, StructType, Table, TableValue, Warehouse,
};

/// A scratch warehouse folder unique to this test and process, not created.
fn scratch(label: &str) -> PathBuf {
    let mut path = LocalFolder::temporary().unwrap().path().unwrap();
    path.push(format!(
        "yggdryl-iceberg-catalog-{label}-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&path);
    path
}

/// The catalog `lake` over `path`, as the generic enum holds it.
fn lake(path: &Path) -> Catalog {
    Catalog::from(IcebergCatalog::bound(
        "lake",
        Holder::LocalFolder(LocalFolder::new(path).unwrap()),
    ))
}

/// Build the catalog `lake` over a scratch warehouse, touching nothing.
fn warehouse(label: &str) -> (PathBuf, Catalog) {
    let path = scratch(label);
    let catalog = lake(&path);
    (path, catalog)
}

/// The two-column taxi schema the catalog tests write, deliberately
/// unnumbered so the catalog has to number it.
fn taxi_schema() -> Field {
    StructType::from_fields([
        DataType::Int64.required_field("id"),
        DataType::utf8().nullable_field("venue"),
    ])
    .map(DataType::from)
    .unwrap()
    .required_field("row")
}

/// The taxi schema with `venue` marked as its own partition column.
fn marked_taxi_schema() -> Field {
    StructType::from_fields([
        DataType::Int64.required_field("id"),
        DataType::utf8()
            .nullable_field("venue")
            .with_partition(true),
    ])
    .map(DataType::from)
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

/// One reader over one batch.
fn reader(batch: RecordBatch) -> yggdryl::arrow::BatchReader {
    yggdryl::arrow::batch_reader(batch.schema(), [batch])
}

/// Collect every row of a reader as sorted `(id, venue)` pairs.
fn collect(reader: yggdryl::arrow::BatchReader) -> Vec<(i64, Option<String>)> {
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

/// Every row of a table through its generic record surface, sorted.
fn rows(table: &Table) -> Vec<(i64, Option<String>)> {
    let options = table.record_options().unwrap();
    collect(table.read_arrow_reader(&options).unwrap())
}

/// Drain a names iterator, panicking on the first failing entry.
fn names(names: Names) -> Vec<String> {
    names
        .map(|name| name.map(|name| name.to_string()))
        .collect::<yggdryl::Result<Vec<_>>>()
        .unwrap()
}

/// The Iceberg table a generic table holds.
fn iceberg(table: Table) -> IcebergTable<Handle> {
    match table {
        Table::Iceberg(table) => *table,
        other => panic!("expected an Iceberg table, got {other:?}"),
    }
}

/// No properties.
fn none() -> Properties {
    Properties::new()
}

/// The namespace the dotted `path` names, each level opened or created in
/// turn: a create descends through existing namespaces only, so the levels
/// are made before what goes under them.
fn namespaces(catalog: &Catalog, path: &str) -> Namespace {
    let mut dotted = String::new();
    let mut namespace = None;
    for part in path.split('.') {
        if !dotted.is_empty() {
            dotted.push('.');
        }
        dotted.push_str(part);
        namespace = Some(
            catalog
                .namespaces()
                .open_or_create(&dotted, &none())
                .unwrap(),
        );
    }
    namespace.unwrap()
}

/// Create the table the dotted `name` names under `field`, its namespaces
/// first.
fn created_with(catalog: &Catalog, name: &str, field: &Field) -> Table {
    if let Some((parent, _)) = name.rsplit_once('.') {
        namespaces(catalog, parent);
    }
    catalog.tables().create(name, field, &none()).unwrap()
}

/// Create the taxi table the dotted `name` names, its namespaces first.
fn created(catalog: &Catalog, name: &str) -> Table {
    created_with(catalog, name, &taxi_schema())
}

#[test]
fn constructing_a_catalog_touches_nothing() {
    let (path, catalog) = warehouse("lazy");

    // Asking questions of an empty warehouse is answered, not failed, and
    // neither the construction nor the questions bring the folder into being.
    assert_eq!(names(catalog.namespaces().iter()), Vec::<String>::new());
    assert_eq!(names(catalog.tables().iter()), Vec::<String>::new());
    assert!(!catalog.tables().contains("nyc.taxis").unwrap());
    assert_eq!(
        catalog.namespace_levels(),
        None,
        "namespaces nest to any depth"
    );
    assert!(!path.exists());
}

#[test]
fn each_object_answers_the_kind_and_the_implementation_it_is() {
    let (_path, catalog) = warehouse("kinds");

    // Storage sees three folders; the framing is what tells them apart, so
    // each value answers for itself.
    assert_eq!(ObjectValue::kind(&catalog), IOKind::Catalog);
    assert!(matches!(catalog, Catalog::Iceberg(_)));

    let table = created(&catalog, "nyc.taxis");
    assert!(matches!(table, Table::Iceberg(_)), "{table:?}");
    assert_eq!(IOBase::kind(&table), IOKind::Table);
    assert_eq!(ObjectValue::kind(&table), IOKind::Table);
    assert!(table.is_tabular());
    assert!(!table.is_atomic());
    assert_eq!(table.storage(), "table");
    assert_eq!(table.to_string(), "lake.nyc.taxis");
    assert_eq!(table.name(), "taxis");

    // A namespace is a folder that is not a table; that it is a *namespace*
    // is what the catalog framing adds.
    let nyc = catalog.namespaces().get("nyc").unwrap();
    assert!(matches!(nyc, Namespace::Iceberg(_)), "{nyc:?}");
    assert_eq!(ObjectValue::kind(&nyc), IOKind::Namespace);
    assert_eq!(nyc.to_string(), "lake.nyc");
    assert_eq!(nyc.name(), "nyc");
}

#[test]
fn a_table_created_through_a_dotted_name_round_trips_its_rows() {
    let (_path, catalog) = warehouse("round-trip");

    // Two namespace levels deep, from one dotted name.
    let table = created(&catalog, "nyc.yellow.taxis");
    assert!(iceberg(table).current_snapshot().unwrap().is_none());
    assert!(catalog.tables().contains("nyc.yellow.taxis").unwrap());
    assert!(!catalog.tables().contains("nyc.green.taxis").unwrap());

    catalog
        .tables()
        .append_arrow_reader(
            "nyc.yellow.taxis",
            reader(taxis(&[1, 2], &[Some("XNAS"), None])),
        )
        .unwrap();

    let table = catalog.table("nyc.yellow.taxis").unwrap();
    assert_eq!(rows(&table), [(1, Some("XNAS".to_owned())), (2, None)]);
    assert_eq!(
        collect(iceberg(table).scan(None).unwrap()),
        [(1, Some("XNAS".to_owned())), (2, None)],
        "the generic surface and the table's own scan read the same rows"
    );
}

#[test]
fn the_cascade_and_the_dotted_name_reach_the_same_table() {
    let (_path, catalog) = warehouse("cascade-equality");
    created(&catalog, "sales.eu.orders");

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
    assert_eq!(
        cascaded, dotted,
        "one description, whichever way it was reached"
    );
    assert_eq!(cascaded, entry);

    let uuid = iceberg(cascaded)
        .metadata()
        .unwrap()
        .table_uuid()
        .to_owned();
    assert_eq!(iceberg(dotted).metadata().unwrap().table_uuid(), uuid);
    assert_eq!(iceberg(entry).metadata().unwrap().table_uuid(), uuid);

    // And a dotted namespace name descends exactly as the cascade does.
    let eu = catalog.namespaces().get("sales.eu").unwrap();
    assert_eq!(eu.name(), "eu");
    assert_eq!(eu.to_string(), "lake.sales.eu");
    assert!(eu.tables().contains("orders").unwrap());
}

#[test]
fn a_create_descends_through_existing_namespaces_only() {
    let (path, catalog) = warehouse("ancestry");

    // A table under namespaces that are not there is the typed absence of
    // the first missing one - the one rule every catalog follows - and the
    // refusal writes nothing.
    let absent = catalog
        .tables()
        .create("a.b.c.orders", &taxi_schema(), &none())
        .unwrap_err();
    assert!(absent.is_absent(), "{absent}");
    assert!(absent.to_string().contains("lake.a"), "{absent}");
    assert!(!path.exists());

    // Each level is one create, and the table's own metadata write is what
    // brings its folder into being.
    let table = created(&catalog, "a.b.c.orders");
    assert!(iceberg(table).current_snapshot().unwrap().is_none());
    assert!(path.join("a/b/c/orders/metadata").is_dir());
    assert!(path.join("a/b/c/metadata/namespace.json").is_file());

    // The table opens through every spelling, and so does each ancestor.
    assert!(catalog.table("a.b.c.orders").is_ok());
    assert!(catalog.namespaces().contains("a").unwrap());
    assert!(catalog.namespaces().get("a.b.c").is_ok());
    assert!(catalog.namespace("a.b").is_ok());
}

#[test]
fn create_table_derives_the_spec_and_numbers_an_unnumbered_schema() {
    let (_path, catalog) = warehouse("marked-schema");

    let table = iceberg(created_with(&catalog, "nyc.taxis", &marked_taxi_schema()));

    // The schema's own partition mark became the table's default spec.
    let metadata = table.metadata().unwrap();
    let spec = metadata.default_spec().unwrap();
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
    assert_eq!(
        TableValue::field(&table).unwrap(),
        *schema,
        "the generic field is the metadata's schema"
    );

    // A schema that marks nothing produces the unpartitioned spec.
    let plain = iceberg(created(&catalog, "nyc.plain"));
    assert!(
        plain
            .metadata()
            .unwrap()
            .default_spec()
            .unwrap()
            .is_unpartitioned()
    );
}

#[test]
fn a_create_through_the_catalog_takes_the_schema_as_iceberg_expresses_it() {
    let (_path, catalog) = warehouse("widened");
    namespaces(&catalog, "nyc");

    // A dictionary-encoded string is an Arrow layout, not an Iceberg type:
    // the table is created over the string it encodes, and the rows that
    // arrive dictionary-encoded are cast to it on the way in.
    let encoded = StructType::from_fields([
        DataType::Int64.required_field("id"),
        DataType::dictionary(DataType::Int32, DataType::utf8())
            .unwrap()
            .nullable_field("venue"),
    ])
    .map(DataType::from)
    .unwrap()
    .required_field("row");
    let table = catalog
        .tables()
        .create("nyc.encoded", &encoded, &none())
        .unwrap();
    let stored = TableValue::field(&table).unwrap();
    assert_eq!(
        stored.get_field_by_path("venue").unwrap().dtype(),
        &DataType::utf8()
    );
    let batch = RecordBatch::try_new(
        encoded.into_arrow_schema().unwrap(),
        vec![
            Arc::new(Int64Array::from(vec![1, 2])),
            Arc::new(
                arrow_array::DictionaryArray::<arrow_array::types::Int32Type>::from_iter([
                    Some("XNAS"),
                    None,
                ]),
            ),
        ],
    )
    .unwrap();
    let mut table = table;
    table
        .append_arrow_reader(reader(batch), &table.record_options().unwrap())
        .unwrap();
    assert_eq!(rows(&table), [(1, Some("XNAS".to_owned())), (2, None)]);

    // A width Iceberg lacks is widened to the one it has: `float16` is
    // stored as `float`.
    let half = catalog
        .tables()
        .create(
            "nyc.half",
            &StructType::from_fields([DataType::Float16.required_field("x")])
                .map(DataType::from)
                .unwrap()
                .required_field("row"),
            &none(),
        )
        .unwrap();
    assert_eq!(
        TableValue::field(&half)
            .unwrap()
            .get_field_by_path("x")
            .unwrap()
            .dtype(),
        &DataType::Float32
    );

    // A column no Iceberg type holds is refused by path, and nothing is
    // created.
    let error = catalog
        .tables()
        .create(
            "nyc.unholdable",
            &StructType::from_fields([DataType::interval(yggdryl::TimeUnit::MonthDayNano)
                .unwrap()
                .required_field("x")])
            .map(DataType::from)
            .unwrap()
            .required_field("row"),
            &none(),
        )
        .unwrap_err();
    assert_eq!(
        error.to_string(),
        "invalid IcebergCompatibility datatype: $.row.x: Iceberg has no calendar interval type, got interval(month_day_nano)"
    );
    assert!(!catalog.tables().contains("nyc.unholdable").unwrap());
}

#[test]
fn append_creates_on_first_write_and_appends_on_the_second() {
    let (_path, catalog) = warehouse("append-creates");
    namespaces(&catalog, "ops");

    let table = catalog
        .tables()
        .append_arrow_reader("ops.trips", reader(taxis(&[1], &[Some("XNAS")])))
        .unwrap();

    // The inferred schema is the reader's, numbered before it was written.
    let schema = TableValue::field(&table).unwrap();
    assert_eq!(
        schema
            .get_field_by_path("id")
            .unwrap()
            .parquet_field_id()
            .unwrap(),
        Some(1)
    );

    let appended = catalog
        .tables()
        .append_arrow_reader("ops.trips", reader(taxis(&[2], &[None])))
        .unwrap();

    // The returned table and a fresh open through the name read the same rows.
    let expected = [(1, Some("XNAS".to_owned())), (2, None)];
    assert_eq!(rows(&appended), expected);
    assert_eq!(rows(&catalog.table("ops.trips").unwrap()), expected);
}

#[test]
fn append_takes_the_partition_marks_that_survived_the_arrow_round_trip() {
    let (_path, catalog) = warehouse("append-marked");
    namespaces(&catalog, "nyc");

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
            yggdryl::arrow::batch_reader(arrow_schema, [batch]),
        )
        .unwrap();

    assert_eq!(rows(&table).len(), 2);
    let table = iceberg(table);
    let metadata = table.metadata().unwrap();
    let spec = metadata.default_spec().unwrap();
    assert_eq!(spec.fields.len(), 1);
    assert_eq!(spec.fields[0].name.as_str(), "venue");
}

#[test]
fn overwrite_creates_when_absent_and_replaces_when_present() {
    let (_path, catalog) = warehouse("overwrite");
    namespaces(&catalog, "nyc");

    catalog
        .tables()
        .overwrite_arrow_reader(
            "nyc.taxis",
            reader(taxis(&[1, 2], &[Some("XNAS"), Some("XNYS")])),
        )
        .unwrap();
    let table = catalog
        .tables()
        .overwrite_arrow_reader("nyc.taxis", reader(taxis(&[9], &[None])))
        .unwrap();

    assert_eq!(rows(&table), [(9, None)]);
}

#[test]
fn absence_and_conflict_are_typed_at_every_level() {
    let (_path, catalog) = warehouse("typed-failures");
    created(&catalog, "nyc.taxis");

    // A create over an existing table is the typed conflict, from the same
    // one classification the open paths use - never from a separate probe.
    let conflict = catalog
        .tables()
        .create("nyc.taxis", &taxi_schema(), &none())
        .unwrap_err();
    assert!(conflict.is_conflict(), "{conflict}");
    assert!(
        conflict.to_string().contains("lake.nyc.taxis"),
        "the full dotted path is named: {conflict}"
    );

    // A missing table is the typed absence, naming the full dotted path.
    let absent = catalog.table("nyc.cabs").unwrap_err();
    assert!(absent.is_absent(), "{absent}");
    assert!(absent.to_string().contains("lake.nyc.cabs"), "{absent}");

    // The same two shapes one level up.
    catalog.namespaces().create("sales", &none()).unwrap();
    let conflict = catalog.namespaces().create("sales", &none()).unwrap_err();
    assert!(conflict.is_conflict(), "{conflict}");
    let absent = catalog.namespaces().get("ops").unwrap_err();
    assert!(absent.is_absent(), "{absent}");

    // And a table's name conflicts with a namespace create, naming both.
    let message = catalog
        .namespaces()
        .create("nyc.taxis", &none())
        .unwrap_err();
    assert!(message.is_conflict(), "{message}");
    assert!(message.to_string().contains("table"), "{message}");

    // A table met on the way down is the absence of the namespace below it.
    let absent = catalog.namespace("nyc.taxis.deeper").unwrap_err();
    assert!(absent.is_absent(), "{absent}");
}

#[test]
fn a_bad_name_segment_is_refused_by_name() {
    let (path, catalog) = warehouse("bad-names");

    let message = catalog
        .tables()
        .create("", &taxi_schema(), &none())
        .unwrap_err()
        .to_string();
    assert!(message.contains("empty"), "{message}");

    let message = catalog
        .tables()
        .contains("nyc..taxis")
        .unwrap_err()
        .to_string();
    assert!(message.contains("nyc..taxis"), "{message}");

    // Dotted text is read by the path grammar, which refuses what it cannot
    // read; parts given as they are reach the catalog's own rule, which
    // names what a part may not be.
    let message = catalog
        .tables()
        .create("a=b", &taxi_schema(), &none())
        .unwrap_err()
        .to_string();
    assert!(message.contains("a=b"), "{message}");
    let message = catalog.resolve(["a=b"]).unwrap_err().to_string();
    assert!(message.contains("a=b"), "{message}");
    assert!(message.contains("partition directory"), "{message}");
    let message = catalog.resolve(["nyc/taxis"]).unwrap_err().to_string();
    assert!(message.contains("'/'"), "{message}");

    // The reserved metadata name is refused at every level, because that
    // folder is where each level keeps its own document.
    let message = catalog
        .tables()
        .create("metadata", &taxi_schema(), &none())
        .unwrap_err()
        .to_string();
    assert!(message.contains("metadata"), "{message}");
    let message = catalog
        .namespaces()
        .create("metadata", &none())
        .unwrap_err()
        .to_string();
    assert!(message.contains("metadata"), "{message}");

    // A refused name creates nothing.
    assert!(!path.exists());
}

#[test]
fn listing_sees_what_was_created_and_a_stray_folder_is_a_namespace() {
    let (path, catalog) = warehouse("listing");
    for name in ["nyc.taxis", "nyc.cabs", "ops.trips"] {
        created(&catalog, name);
    }

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

    // The children walk the same level as one lazy listing, in name order.
    let children: Vec<String> = nyc
        .children()
        .map(|child| child.map(|child| child.to_string()))
        .collect::<yggdryl::Result<_>>()
        .unwrap();
    assert_eq!(
        children,
        ["lake.nyc.cabs", "lake.nyc.scratch", "lake.nyc.taxis"]
    );
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
    created(&catalog, "nyc.taxis");
    assert_eq!(names(namespaces.iter()), ["nyc"]);
    assert!(namespaces.contains("nyc").unwrap());
    let nyc = namespaces.get("nyc").unwrap();
    assert_eq!(names(nyc.tables().iter()), ["taxis"]);

    // Two views over the same catalog observe each other's writes.
    let tables = nyc.tables();
    let second = catalog.namespaces().get("nyc").unwrap();
    second
        .tables()
        .create("cabs", &taxi_schema(), &none())
        .unwrap();
    assert_eq!(names(tables.iter()), ["cabs", "taxis"]);
    assert_eq!(tables.len().unwrap(), 2);
    assert!(tables.contains("cabs").unwrap());
}

#[test]
fn access_chains_through_the_views_and_a_missing_name_is_named() {
    let (_path, catalog) = warehouse("chained-views");
    created(&catalog, "nyc.yellow.taxis");

    // The cascade: a nested namespace is reached through its parent's view.
    let nyc = catalog.namespaces().get("nyc").unwrap();
    let yellow = nyc.namespaces().get("yellow").unwrap();
    assert_eq!(yellow.to_string(), "lake.nyc.yellow");
    let table = yellow.tables().get("taxis").unwrap();
    assert!(iceberg(table).current_snapshot().unwrap().is_none());

    // A missing table is a typed error naming the full dotted path.
    let message = yellow.tables().get("cabs").unwrap_err().to_string();
    assert!(message.contains("lake.nyc.yellow.cabs"), "{message}");

    // A missing namespace is a typed error naming the namespace, and a
    // table's name is not a namespace.
    let message = catalog.namespaces().get("ops").unwrap_err().to_string();
    assert!(message.contains("lake.ops"), "{message}");
    assert!(!nyc.namespaces().contains("taxis").unwrap());
    let error = nyc.namespaces().get("yellow.taxis").unwrap_err();
    assert!(error.is_absent(), "{error}");
}

#[test]
fn an_empty_namespace_is_durable_and_survives_a_reopen() {
    let (path, catalog) = warehouse("durable-namespace");

    // The namespace document is what makes an empty one durable - no marker
    // trick, no zero-byte truncation.
    let sales = catalog.namespaces().create("sales", &none()).unwrap();
    assert_eq!(sales.to_string(), "lake.sales");
    assert!(path.join("sales/metadata/namespace.json").is_file());

    // A second catalog over the same folder sees it, holding no tables.
    let reopened = lake(&path);
    assert!(reopened.namespaces().contains("sales").unwrap());
    let sales = reopened.namespaces().get("sales").unwrap();
    assert!(sales.tables().is_empty().unwrap());
    assert_eq!(names(reopened.namespaces().iter()), ["sales"]);

    // Its own metadata folder is infrastructure, never a child namespace.
    assert_eq!(names(sales.namespaces().iter()), Vec::<String>::new());
}

#[test]
fn a_folder_of_warehouses_is_registered_catalog_by_catalog() {
    let root = scratch("catalogs");
    let folder = |name: &str| Holder::LocalFolder(LocalFolder::new(root.join(name)).unwrap());

    // Creating a catalog writes its document, and that is what creates the
    // folder: the second create over it is the typed conflict, and
    // `open_or_create` absorbs it.
    let lake = IcebergCatalog::create("lake", folder("lake")).unwrap();
    assert!(root.join("lake/metadata/catalog.json").is_file());
    let conflict = IcebergCatalog::create("lake", folder("lake")).unwrap_err();
    assert!(conflict.is_conflict(), "{conflict}");
    let again = IcebergCatalog::open_or_create("lake", folder("lake")).unwrap();
    assert_eq!(again, lake);
    let pond = IcebergCatalog::open_or_create("pond", folder("pond")).unwrap();
    assert!(root.join("pond/metadata/catalog.json").is_file());

    // A table's folder is no catalog, and says so.
    created(&Catalog::from(lake.clone()), "sales.orders");
    let refused = IcebergCatalog::open_or_create("orders", folder("lake/sales/orders"))
        .unwrap_err()
        .to_string();
    assert!(refused.contains("got a table"), "{refused}");

    // Each catalog is registered under its own name, so a folder of
    // warehouses is as many catalogs as it holds folders.
    let mut warehouse = Warehouse::default();
    warehouse.register(lake).unwrap();
    warehouse.register(pond).unwrap();
    assert_eq!(
        warehouse
            .catalogs()
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>(),
        ["lake", "pond"]
    );
    assert!(warehouse.table("lake.sales.orders").is_ok());
    assert!(
        warehouse
            .table("pond.sales.orders")
            .unwrap_err()
            .is_absent()
    );
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn properties_round_trip_at_all_three_levels() {
    let root = scratch("properties");
    let lake = IcebergCatalog::create(
        "lake",
        Holder::LocalFolder(LocalFolder::new(root.join("lake")).unwrap()),
    )
    .unwrap();

    // Catalog properties live in metadata/catalog.json under the warehouse.
    assert!(lake.properties().unwrap().is_empty());
    lake.update_properties(&Properties::new().with_property("owner", "ops"), &[])
        .unwrap();
    assert_eq!(lake.properties().unwrap().get("owner"), Some("ops"));

    // Namespace properties live in metadata/namespace.json under the folder,
    // beneath the catalog's, which every child inherits.
    let catalog = Catalog::from(lake.clone());
    let sales = catalog.namespaces().create("sales", &none()).unwrap();
    assert_eq!(
        sales.properties().unwrap().get("owner"),
        Some("ops"),
        "the parent's effective properties reach the child"
    );
    sales
        .update_properties(
            &Properties::new()
                .with_property("region", "eu")
                .with_property("tier", "gold"),
            &[],
        )
        .unwrap();
    sales.update_properties(&none(), &["tier".into()]).unwrap();
    let properties = sales.properties().unwrap();
    assert_eq!(properties.get("region"), Some("eu"));
    assert_eq!(properties.get("owner"), Some("ops"));
    assert!(properties.get("tier").is_none());

    // The reserved prefix is refused by name, and the refusal changes nothing.
    let refused = sales
        .update_properties(&Properties::new().with_property("ICEBERG:spec", "x"), &[])
        .unwrap_err()
        .to_string();
    assert!(refused.contains("ICEBERG:"), "{refused}");
    assert_eq!(sales.properties().unwrap().len(), 2);

    // Table properties ride the metadata document; what was stated for the
    // table at creation is answered over them and written nowhere.
    let table = catalog
        .tables()
        .create(
            "sales.orders",
            &taxi_schema(),
            &Properties::new().with_property("write.format.default", "parquet"),
        )
        .unwrap();
    let properties = table.properties().unwrap();
    assert_eq!(properties.get("owner"), Some("ops"));
    assert_eq!(properties.get("region"), Some("eu"));
    assert_eq!(properties.get("write.format.default"), Some("parquet"));
    let table = iceberg(table);
    assert!(table.metadata().unwrap().properties().is_empty());
    let refused = table
        .update_properties(&Properties::new().with_property("owner", "me"), &[])
        .unwrap_err()
        .to_string();
    assert!(refused.contains("commit_metadata_changes"), "{refused}");

    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn an_absent_properties_document_answers_empty_at_every_level() {
    let (path, catalog) = warehouse("absent-properties");

    // A fresh catalog over an empty warehouse: no folder, no document, and
    // still an answer - absent means empty, never a missing-file failure.
    assert!(catalog.properties().unwrap().is_empty());
    assert!(!path.exists());

    // A folder somebody else made is a namespace with no document of its
    // own, and it answers the same way - and a table is created under it.
    std::fs::create_dir_all(path.join("nyc")).unwrap();
    assert!(!path.join("nyc/metadata/namespace.json").exists());
    let nyc = catalog.namespaces().get("nyc").unwrap();
    assert!(nyc.properties().unwrap().is_empty());
    let table = catalog
        .tables()
        .create("nyc.taxis", &taxi_schema(), &none())
        .unwrap();
    assert!(table.properties().unwrap().is_empty());
    assert!(!path.join("nyc/metadata/namespace.json").exists());
}

#[test]
fn a_malformed_properties_document_is_refused_naming_what_was_found() {
    let (path, catalog) = warehouse("malformed-properties");
    catalog.namespaces().create("sales", &none()).unwrap();
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
        .update_properties(&Properties::new().with_property("ICEBERG:spec", "x"), &[])
        .unwrap_err()
        .to_string();
    assert!(refused.contains("reserved"), "{refused}");
    assert!(refused.contains("ICEBERG:"), "{refused}");
    assert!(catalog.properties().unwrap().is_empty());
}

#[test]
fn a_catalog_is_named_and_located_as_it_was_stated() {
    let path = scratch("named");
    let url = yggdryl::Url::from_path(&path).unwrap();

    // The name is stated, never read off the folder; the location is the
    // URL it was given, answered from the description with no I/O.
    let catalog = IcebergCatalog::new("lake", url.clone())
        .unwrap()
        .with_description("the lake");
    assert_eq!(catalog.name(), "lake");
    assert_eq!(catalog.path(), ["lake"]);
    assert_eq!(catalog.description(), Some("the lake"));
    assert_eq!(ObjectValue::url(&catalog), Some(&url));
    assert_eq!(catalog.namespace_levels(), None);
    assert!(!path.exists());

    // The `hadoop` type names the same implementation from a URL.
    let built = Catalog::from_url(
        &url,
        &Properties::new()
            .with_property("type", "hadoop")
            .with_property("name", "lake"),
    )
    .unwrap();
    assert!(matches!(built, Catalog::Iceberg(_)), "{built:?}");
    assert_eq!(built.name(), "lake");
    assert_eq!(
        built.properties().unwrap().get("type"),
        Some("hadoop"),
        "every property travels on"
    );
    assert!(!path.exists());
}

#[test]
fn two_creators_of_one_table_converge_or_one_gets_the_typed_conflict() {
    let (path, catalog) = warehouse("racing-creates");
    namespaces(&catalog, "race");

    // Two threads, two catalogs, one name. Storage has no compare-and-swap,
    // so the contract is: both converge on the same table, or one of them
    // gets the typed conflict - never corruption, never a silent third state.
    let barrier = std::sync::Barrier::new(2);
    fn make(path: &Path, barrier: &std::sync::Barrier) -> yggdryl::Result<()> {
        let catalog = lake(path);
        barrier.wait();
        catalog
            .tables()
            .create("race.orders", &taxi_schema(), &none())
            .map(|_| ())
    }
    let outcomes: Vec<yggdryl::Result<()>> = std::thread::scope(|scope| {
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
                    || matches!(error, yggdryl::Error::Codec { .. } | yggdryl::Error::Io(_)),
                "{error}"
            );
        }
    }

    // Whoever won, the table is whole and opens.
    let table = lake(&path).table("race.orders").unwrap();
    assert!(iceberg(table).current_snapshot().unwrap().is_none());
}

#[test]
fn table_writes_through_the_views_create_on_first_write() {
    let (_path, catalog) = warehouse("view-writes");

    let sales = catalog
        .namespaces()
        .open_or_create("sales", &none())
        .unwrap();
    let table = sales
        .tables()
        .append_arrow_reader("orders", reader(taxis(&[1, 2], &[Some("XNAS"), None])))
        .unwrap();
    assert_eq!(rows(&table).len(), 2);

    // The overwrite convenience replaces through the same view.
    let table = sales
        .tables()
        .overwrite_arrow_reader("orders", reader(taxis(&[9], &[None])))
        .unwrap();
    assert_eq!(rows(&table), [(9, None)]);

    // The dotted entry point is the same implementation, so both spellings
    // observe the same table.
    assert_eq!(rows(&catalog.table("sales.orders").unwrap()), [(9, None)]);
}

#[test]
fn a_registered_catalog_answers_its_tables_to_the_warehouse_and_to_a_plan() {
    let (_path, catalog) = warehouse("registered");
    namespaces(&catalog, "nyc");
    catalog
        .tables()
        .append_arrow_reader(
            "nyc.taxis",
            reader(taxis(&[1, 2, 3], &[Some("XNAS"), None, Some("XNYS")])),
        )
        .unwrap();

    let mut warehouse = Warehouse::default();
    warehouse.register(catalog).unwrap();
    let table = warehouse.table("lake.nyc.taxis").unwrap();
    assert!(matches!(table, Table::Iceberg(_)), "{table:?}");
    assert_eq!(rows(&table).len(), 3);

    // A plan names the table by its path and reads it through the same
    // object, its `where` pushed into the scan.
    let plan: yggdryl::expression::Plan = "select id from lake.nyc.taxis where id > 1"
        .parse()
        .unwrap();
    let read: usize = plan
        .execute_in(&warehouse)
        .unwrap()
        .map(|batch| batch.map(|batch| batch.num_rows()))
        .sum::<Result<usize, _>>()
        .unwrap();
    assert_eq!(read, 2);
}

#[test]
fn a_clone_is_the_same_description_over_a_handle_rebuilt_from_the_site() {
    let (_path, catalog) = warehouse("clone");
    namespaces(&catalog, "nyc");
    catalog
        .tables()
        .append_arrow_reader("nyc.taxis", reader(taxis(&[1, 2], &[Some("XNAS"), None])))
        .unwrap();
    let table = catalog.table("nyc.taxis").unwrap();
    assert_eq!(rows(&table).len(), 2);

    // The clone describes the same table and reads the same rows through a
    // handle it rebuilt from the location, under the same properties.
    let twin = table.clone();
    assert_eq!(twin, table);
    assert_eq!(rows(&twin), rows(&table));
    assert_eq!(twin.properties().unwrap(), table.properties().unwrap());

    // A re-stated table is another description.
    let stated = table
        .clone()
        .with_properties(Properties::new().with_property("owner", "ops"));
    assert_ne!(stated, table);
    assert_eq!(stated.properties().unwrap().get("owner"), Some("ops"));
    assert_eq!(rows(&stated).len(), 2);
}

/// The number of backend calls an operation makes is a behavior, not an
/// implementation detail - the expression module's selector cost tests set
/// that precedent, and the existence audit's whole point is the round trips
/// that are no longer spent asking questions whose answers were stale.
mod call_counts {
    use std::sync::Arc;

    use super::{Catalog, IcebergCatalog, created, iceberg, namespaces, none, taxi_schema};
    use crate::counting_filesystem::{CountingFileSystem as Counting, counted_folder};
    use yggdryl::holder::Holder;

    /// A catalog over a counting warehouse, with the counter beside it.
    fn counted() -> (Arc<Counting>, Catalog) {
        let (filesystem, warehouse) = counted_folder("warehouse");
        (
            filesystem,
            Catalog::from(IcebergCatalog::bound(
                "warehouse",
                Holder::FsFolder(warehouse),
            )),
        )
    }

    #[test]
    fn a_get_of_an_existing_table_is_one_resolution_one_listing_and_no_read() {
        let (filesystem, catalog) = counted();
        created(&catalog, "sales.orders");

        // Each level down is one presence answer, one listing of the
        // entry's `metadata/` to tell a table from a namespace, and one read
        // of the level's own document for the properties its child inherits;
        // the table's metadata document is not read until something asks.
        let mut opened = None;
        let calls = filesystem.costs(|| {
            opened = Some(catalog.tables().get("sales.orders").unwrap());
        });
        assert_eq!(
            calls, "file_info=2 list=2 open_input_stream=2",
            "get on an existing table"
        );

        // The first use reads the hint and the document it names; every
        // later one reads nothing.
        let table = iceberg(opened.unwrap());
        let calls = filesystem.costs(|| {
            table.metadata().unwrap();
        });
        assert_eq!(
            calls, "file_info=1 open_input_stream=2",
            "the first use reads the current document"
        );
        let calls = filesystem.costs(|| {
            table.metadata().unwrap();
        });
        assert_eq!(calls, "none", "and the second reads nothing");
    }

    #[test]
    fn a_get_of_a_missing_table_stops_at_the_presence_answer() {
        let (filesystem, catalog) = counted();
        created(&catalog, "sales.orders");

        // The namespace on the way down costs what every level costs; the
        // missing child is one presence answer, so no listing of its
        // `metadata/` runs and nothing is read for it.
        let calls = filesystem.costs(|| {
            catalog.tables().get("sales.nothing").unwrap_err();
        });
        assert_eq!(
            calls, "file_info=2 list=1 open_input_stream=2",
            "get on a missing table"
        );
    }

    #[test]
    fn a_create_under_existing_namespaces_is_their_resolution_and_the_writes() {
        let (filesystem, catalog) = counted();
        namespaces(&catalog, "a.b.c");

        // Three levels down at the cost every level has, the fourth read the
        // namespace's own document for what the table inherits, then the
        // create's own operations: one presence answer, the metadata folder
        // listed once, and the writes - the first output open reports a
        // missing parent, one recursive create repairs it, and the output
        // open is retried exactly once. Nothing walks the ancestry twice.
        let calls = filesystem.costs(|| {
            catalog
                .tables()
                .create("a.b.c.orders", &taxi_schema(), &none())
                .unwrap();
        });
        assert_eq!(
            calls,
            "create_dir=1 delete_file=1 file_info=4 list=4 open_input_stream=4 \
             open_output_stream=4",
            "create under three namespace levels"
        );

        // And the table it made opens.
        catalog.tables().get("a.b.c.orders").unwrap();
    }

    #[test]
    fn open_or_create_costs_the_same_one_classification_on_both_branches() {
        let (filesystem, catalog) = counted();
        namespaces(&catalog, "sales");

        // The absent branch: the get's descent, which ends at the missing
        // child's presence answer, then the create's own descent and writes.
        let absent = filesystem.costs(|| {
            catalog
                .tables()
                .open_or_create("sales.orders", &taxi_schema(), &none())
                .unwrap();
        });
        assert_eq!(
            absent,
            "create_dir=1 delete_file=1 file_info=4 list=3 open_input_stream=4 \
             open_output_stream=4",
            "open_or_create when absent"
        );

        // The present branch: exactly what `get` costs, because it is the
        // same attempt.
        let present = filesystem.costs(|| {
            catalog
                .tables()
                .open_or_create("sales.orders", &taxi_schema(), &none())
                .unwrap();
        });
        assert_eq!(
            present, "file_info=2 list=2 open_input_stream=2",
            "open_or_create when present"
        );
    }
}
