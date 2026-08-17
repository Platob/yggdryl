//! The storage catalog: namespaces of tables under one warehouse folder.

use std::sync::Arc;

use arrow_array::{Array, Int64Array, RecordBatch, StringArray};

use super::Catalog;
use crate::iceberg::Transform;
use crate::local::Folder;
use crate::{DataType, Field};

/// Build a catalog over a scratch warehouse unique to this test and process.
fn warehouse(label: &str) -> (std::path::PathBuf, Catalog<Folder>) {
    let mut path = std::env::temp_dir();
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
    let schema = taxi_schema().to_arrow_schema().unwrap();
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

#[test]
fn constructing_a_catalog_touches_nothing() {
    let (path, catalog) = warehouse("lazy");

    // Asking questions of an empty warehouse is answered, not failed, and
    // neither the construction nor the questions bring the folder into being.
    assert_eq!(catalog.list_namespaces(None).unwrap(), Vec::<String>::new());
    assert_eq!(catalog.list_tables("nyc").unwrap(), Vec::<String>::new());
    assert!(!catalog.has_table("nyc.taxis").unwrap());
    assert!(!path.exists());
}

#[test]
fn a_table_created_through_a_dotted_name_round_trips_its_rows() {
    let (_path, catalog) = warehouse("round-trip");

    // Two namespace levels deep, from one dotted name.
    let table = catalog
        .create_table("nyc.yellow.taxis", taxi_schema())
        .unwrap();
    assert!(table.current_snapshot().is_none());
    assert!(catalog.has_table("nyc.yellow.taxis").unwrap());
    assert!(!catalog.has_table("nyc.green.taxis").unwrap());

    let batch = taxis(&[1, 2], &[Some("XNAS"), None]);
    catalog
        .append(
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
fn create_table_derives_the_spec_and_numbers_an_unnumbered_schema() {
    let (_path, catalog) = warehouse("marked-schema");

    let table = catalog
        .create_table("nyc.taxis", marked_taxi_schema())
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
            .get_field_by_name("id")
            .unwrap()
            .parquet_field_id()
            .unwrap(),
        Some(1)
    );
    assert_eq!(
        schema
            .get_field_by_name("venue")
            .unwrap()
            .parquet_field_id()
            .unwrap(),
        Some(2)
    );
    assert_eq!(spec.fields[0].source_id, 2);

    // A schema that marks nothing produces the unpartitioned spec.
    let plain = catalog.create_table("nyc.plain", taxi_schema()).unwrap();
    assert!(plain.metadata().default_spec().unwrap().is_unpartitioned());
}

#[test]
fn append_creates_on_first_write_and_appends_on_the_second() {
    let (_path, catalog) = warehouse("append-creates");

    let first = taxis(&[1], &[Some("XNAS")]);
    let table = catalog
        .append(
            "ops.trips",
            crate::arrow::batch_reader(first.schema(), [first]),
        )
        .unwrap();

    // The inferred schema is the reader's, rooted at `row` and numbered.
    let schema = table.schema().unwrap();
    assert_eq!(schema.name(), "row");
    assert_eq!(
        schema
            .get_field_by_name("id")
            .unwrap()
            .parquet_field_id()
            .unwrap(),
        Some(1)
    );

    let second = taxis(&[2], &[None]);
    let appended = catalog
        .append(
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
    let arrow_schema = marked_taxi_schema().to_arrow_schema().unwrap();
    let batch = RecordBatch::try_new(
        Arc::clone(&arrow_schema),
        vec![
            Arc::new(Int64Array::from(vec![1, 2])),
            Arc::new(StringArray::from(vec![Some("XNAS"), Some("XNYS")])),
        ],
    )
    .unwrap();
    let table = catalog
        .append(
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
        .overwrite(
            "nyc.taxis",
            crate::arrow::batch_reader(first.schema(), [first]),
        )
        .unwrap();

    let second = taxis(&[9], &[None]);
    let table = catalog
        .overwrite(
            "nyc.taxis",
            crate::arrow::batch_reader(second.schema(), [second]),
        )
        .unwrap();

    assert_eq!(collect(table.scan(None).unwrap()), [(9, None)]);
}

#[test]
fn an_existing_table_refuses_create_and_a_missing_one_refuses_open() {
    let (_path, catalog) = warehouse("refusals");
    catalog.create_table("nyc.taxis", taxi_schema()).unwrap();

    let message = catalog
        .create_table("nyc.taxis", taxi_schema())
        .unwrap_err()
        .to_string();
    assert!(
        message.contains("expected no table at \"nyc.taxis\", got one; open it with table"),
        "{message}"
    );

    let message = catalog.table("nyc.cabs").unwrap_err().to_string();
    assert!(
        message.contains("expected a table at \"nyc.cabs\""),
        "{message}"
    );
}

#[test]
fn a_bad_name_segment_is_refused_by_name() {
    let (path, catalog) = warehouse("bad-names");

    let message = catalog
        .create_table("", taxi_schema())
        .unwrap_err()
        .to_string();
    assert!(message.contains("got an empty one"), "{message}");

    let message = catalog.has_table("nyc..taxis").unwrap_err().to_string();
    assert!(message.contains("\"nyc..taxis\""), "{message}");

    let message = catalog
        .create_table("a=b", taxi_schema())
        .unwrap_err()
        .to_string();
    assert!(message.contains("\"a=b\""), "{message}");
    assert!(message.contains("partition directory"), "{message}");

    let message = catalog.list_tables("nyc/taxis").unwrap_err().to_string();
    assert!(message.contains("'/'"), "{message}");

    // A refused name creates nothing.
    assert!(!path.exists());
}

#[test]
fn listing_sees_what_was_created_and_a_stray_folder_is_a_namespace() {
    let (path, catalog) = warehouse("listing");
    catalog.create_table("nyc.taxis", taxi_schema()).unwrap();
    catalog.create_table("nyc.cabs", taxi_schema()).unwrap();
    catalog.create_table("ops.trips", taxi_schema()).unwrap();

    // A plain folder somebody else made is a namespace, never a table, and a
    // stray file is neither.
    std::fs::create_dir_all(path.join("nyc").join("scratch")).unwrap();
    std::fs::write(path.join("notes.txt"), b"not a namespace").unwrap();

    assert_eq!(catalog.list_namespaces(None).unwrap(), ["nyc", "ops"]);
    assert_eq!(
        catalog.list_namespaces(Some("nyc")).unwrap(),
        ["nyc.scratch"]
    );
    assert_eq!(
        catalog.list_tables("nyc").unwrap(),
        ["nyc.cabs", "nyc.taxis"]
    );
    assert_eq!(catalog.list_tables("ops").unwrap(), ["ops.trips"]);
    assert_eq!(
        catalog.list_tables("nyc.scratch").unwrap(),
        Vec::<String>::new()
    );
    assert_eq!(
        catalog.list_namespaces(Some("ops")).unwrap(),
        Vec::<String>::new()
    );
}
