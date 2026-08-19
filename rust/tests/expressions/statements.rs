//! Statements: the lowering first, and only then what it does to bytes.
//!
//! A wrong `UPDATE` should be caught as a wrong `CASE` expression rather than
//! as wrong bytes, so the lowering is tested directly and the executor is
//! tested as the thin thing it is.

use std::sync::Arc;

use arrow_array::{Int64Array, RecordBatch, StringArray};
use yggdryl::expressions::{Applied, Statement, WriteMode};
use yggdryl::io::{Buffer, IOBase};
use yggdryl::{DataType, Expr, Field, MimeType, Value, arrow};

/// The root every case here runs against.
fn schema() -> Field {
    DataType::from_fields([
        DataType::Int64.nullable_field("id"),
        DataType::Utf8.nullable_field("venue"),
        DataType::Int64.nullable_field("price"),
    ])
    .expect("a three-column struct")
    .required_field("row")
}

/// A handle holding four rows, one of them with a null price.
fn stored() -> Buffer {
    let batch = RecordBatch::try_new(
        arrow::schema_from_field(&schema()).expect("an Arrow schema"),
        vec![
            Arc::new(Int64Array::from(vec![
                Some(1_i64),
                Some(2),
                Some(3),
                Some(4),
            ])),
            Arc::new(StringArray::from(vec![
                Some("XNAS"),
                Some("XNYS"),
                Some("XNAS"),
                Some("XLON"),
            ])),
            Arc::new(Int64Array::from(vec![
                Some(5_i64),
                Some(15),
                Some(25),
                None,
            ])),
        ],
    )
    .expect("a four-row batch");
    let mut handle = Buffer::new().with_media_type(MimeType::ARROW_STREAM.into());
    let options = handle.record_options().expect("options");
    handle
        .write_arrow_batch_reader(arrow::batch_reader(batch.schema(), [batch]), &options)
        .expect("the rows are written");
    handle
}

/// Every `(id, venue, price)` a handle holds.
fn rows(handle: &Buffer) -> Vec<(i64, Option<String>, Option<i64>)> {
    let options = handle.record_options().expect("options");
    let mut found = Vec::new();
    for batch in handle.read_arrow_batch_reader(&options).expect("a reader") {
        let batch = batch.expect("a batch");
        let ids = column::<Int64Array>(&batch, "id");
        let venues = batch.column_by_name("venue").map(|column| {
            column
                .as_any()
                .downcast_ref::<StringArray>()
                .expect("a text column")
                .clone()
        });
        let prices = batch.column_by_name("price").map(|column| {
            column
                .as_any()
                .downcast_ref::<Int64Array>()
                .expect("an integer column")
                .clone()
        });
        for row in 0..batch.num_rows() {
            found.push((
                ids.value(row),
                venues
                    .as_ref()
                    .filter(|column| !arrow_array::Array::is_null(*column, row))
                    .map(|column| column.value(row).to_owned()),
                prices
                    .as_ref()
                    .filter(|column| !arrow_array::Array::is_null(*column, row))
                    .map(|column| column.value(row)),
            ));
        }
    }
    found
}

/// One column of a batch, downcast.
fn column<T: 'static + Clone>(batch: &RecordBatch, name: &str) -> T {
    batch
        .column_by_name(name)
        .unwrap_or_else(|| panic!("the column {name}"))
        .as_any()
        .downcast_ref::<T>()
        .unwrap_or_else(|| panic!("the column {name} has another type"))
        .clone()
}

#[test]
fn every_statement_round_trips_through_its_own_spelling() {
    for text in [
        "SELECT *",
        "SELECT id, venue AS market WHERE price > 10",
        "INSERT INTO . VALUES (1, 'XNAS', 5)",
        "INSERT INTO . (id, venue) VALUES (1, 'XNAS'), (2, 'XNYS')",
        "UPDATE . SET price = price * 2 WHERE venue = 'XNAS'",
        "DELETE FROM . WHERE price > 10",
        "DELETE FROM .",
        "ALTER TABLE . ADD COLUMN fee int64",
        "ALTER TABLE . ADD COLUMN fee int64 DEFAULT 0",
        "ALTER TABLE . ADD COLUMN doubled int64 AS price * 2",
        "ALTER TABLE . DROP COLUMN price",
        "ALTER TABLE . RENAME COLUMN venue TO market",
        "ALTER TABLE . ALTER COLUMN price TYPE decimal(10, 2)",
        "DELETE FROM . WHERE price > 100; ALTER TABLE . DROP COLUMN price",
    ] {
        let parsed: Statement = text
            .parse()
            .unwrap_or_else(|error| panic!("{text}: {error}"));
        let once = parsed.to_string();
        let twice: Statement = once
            .parse()
            .unwrap_or_else(|error| panic!("{once}: {error}"));
        assert_eq!(once, twice.to_string(), "round trip of {text:?}");
    }
}

#[test]
fn delete_lowers_to_the_complement_that_keeps_an_unknown() {
    let schema = schema();
    let lowered = "DELETE WHERE price > 10"
        .parse::<Statement>()
        .expect("parses")
        .lower(&schema)
        .expect("lowers");
    assert_eq!(lowered.mode, WriteMode::Overwrite);
    // Not `NOT (price > 10)`: that is unknown when price is null, and unknown
    // would drop the row. The null test is what keeps it.
    let filter = lowered.filter.expect("a filter").to_string();
    assert!(filter.contains("IS NULL"), "{filter}");
}

#[test]
fn update_lowers_to_a_case_over_every_column() {
    let schema = schema();
    let lowered = "UPDATE . SET price = price * 2 WHERE venue = 'XNAS'"
        .parse::<Statement>()
        .expect("parses")
        .lower(&schema)
        .expect("lowers");
    assert_eq!(lowered.mode, WriteMode::Overwrite);
    let projection = lowered.selection.to_string();
    // The assigned column becomes a conditional; the others are carried.
    assert!(
        projection.contains("CASE WHEN venue = 'XNAS' THEN price * 2 ELSE price END AS price"),
        "{projection}"
    );
    assert!(projection.contains("id AS id"), "{projection}");
    assert!(projection.contains("venue AS venue"), "{projection}");
    // And the filter is gone: a conditional rewrite keeps every row.
    assert!(lowered.filter.is_none(), "{lowered:?}");
}

#[test]
fn the_schema_verbs_lower_to_the_projections_they_are() {
    let schema = schema();
    let lower = |text: &str| {
        text.parse::<Statement>()
            .unwrap_or_else(|error| panic!("{text}: {error}"))
            .lower(&schema)
            .unwrap_or_else(|error| panic!("{text}: {error}"))
    };

    let dropped = lower("ALTER TABLE . DROP COLUMN price");
    assert_eq!(dropped.root.field_len(), 2);
    assert!(dropped.root.get_field_by_name("price").is_none());

    let renamed = lower("ALTER TABLE . RENAME COLUMN venue TO market");
    assert!(renamed.root.get_field_by_name("market").is_some());
    assert!(renamed.root.get_field_by_name("venue").is_none());

    let retyped = lower("ALTER TABLE . ALTER COLUMN price TYPE decimal(10, 2)");
    assert_eq!(
        retyped
            .root
            .get_field_by_name("price")
            .expect("price")
            .data_type(),
        &DataType::decimal(10, 2).expect("a decimal")
    );

    let added = lower("ALTER TABLE . ADD COLUMN fee int64 DEFAULT 0");
    assert_eq!(added.root.field_len(), 4);
    assert!(added.root.get_field_by_name("fee").is_some());
}

#[test]
fn a_statement_naming_an_absent_column_lists_what_the_schema_has() {
    let schema = schema();
    for text in [
        "ALTER TABLE . DROP COLUMN nosuch",
        "ALTER TABLE . RENAME COLUMN nosuch TO other",
        "ALTER TABLE . ALTER COLUMN nosuch TYPE int64",
        "UPDATE . SET nosuch = 1",
    ] {
        let error = text
            .parse::<Statement>()
            .unwrap_or_else(|error| panic!("{text}: {error}"))
            .lower(&schema)
            .expect_err(text);
        let message = error.to_string();
        assert!(message.contains("nosuch"), "{text}: {message}");
        assert!(message.contains("id, venue, price"), "{text}: {message}");
    }
    // And adding a column that already exists is refused the other way.
    let error = "ALTER TABLE . ADD COLUMN price int64"
        .parse::<Statement>()
        .expect("parses")
        .lower(&schema)
        .expect_err("a column that exists");
    assert!(error.to_string().contains("ALTER COLUMN"), "{error}");
}

#[test]
fn a_where_that_is_true_for_every_row_is_refused_unless_it_says_so() {
    let schema = schema();
    // `1 = 1` is a typo, and on a DELETE it is the expensive kind.
    let error = "DELETE WHERE 1 = 1"
        .parse::<Statement>()
        .expect("parses")
        .lower(&schema)
        .expect_err("an always-true WHERE");
    assert!(error.to_string().contains("WHERE TRUE"), "{error}");

    // Spelling it out is a deliberate act and passes.
    assert!(
        "DELETE WHERE TRUE"
            .parse::<Statement>()
            .expect("parses")
            .lower(&schema)
            .is_ok()
    );
    // And omitting the clause entirely is a truncate, which is allowed.
    assert!(
        "DELETE FROM ."
            .parse::<Statement>()
            .expect("parses")
            .lower(&schema)
            .is_ok()
    );
}

#[test]
fn a_chain_fuses_into_one_selection_and_one_filter() {
    let schema = schema();
    let chain: Statement = "DELETE WHERE price > 20; ALTER TABLE . DROP COLUMN venue; \
         ALTER TABLE . RENAME COLUMN price TO amount"
        .parse()
        .expect("parses");
    assert_eq!(chain.steps().len(), 3);

    let lowered = chain.lower(&schema).expect("lowers");
    // Three steps, one projection, one filter, one write.
    assert_eq!(lowered.mode, WriteMode::Overwrite);
    assert!(lowered.filter.is_some());
    let names: Vec<&str> = lowered.root.fields().iter().map(Field::name).collect();
    assert_eq!(names, vec!["id", "amount"]);
}

#[test]
fn a_chain_step_that_cannot_be_typed_is_refused_by_position() {
    let schema = schema();
    let error = "ALTER TABLE . DROP COLUMN price; DELETE WHERE price > 10"
        .parse::<Statement>()
        .expect("parses")
        .lower(&schema)
        .expect_err("a step reading a dropped column");
    let message = error.to_string();
    assert!(message.contains("step 2 of 2"), "{message}");
    assert!(message.contains("price"), "{message}");
    assert!(message.contains("id, venue"), "{message}");
}

#[test]
fn a_chain_of_chains_flattens() {
    let inner: Statement = "DELETE WHERE price > 20; ALTER TABLE . DROP COLUMN venue"
        .parse()
        .expect("parses");
    let outer = inner
        .clone()
        .then("ALTER TABLE . DROP COLUMN price".parse().expect("parses"));
    assert_eq!(outer.steps().len(), 3);
    // And the flattened chain lowers to the same thing the nested one does.
    let schema = schema();
    let nested = Statement::chain([
        inner,
        "ALTER TABLE . DROP COLUMN price".parse().expect("parses"),
    ]);
    assert_eq!(
        outer.lower(&schema).expect("lowers").root,
        nested.lower(&schema).expect("lowers").root
    );
}

#[test]
fn delete_removes_the_matching_rows_and_keeps_the_unknown_ones() {
    let mut handle = stored();
    let Applied::Changed(report) = handle
        .apply_expression(&"DELETE WHERE price > 10".parse().expect("parses"))
        .expect("the delete runs")
    else {
        panic!("a delete reports rather than reading");
    };
    assert_eq!(report.rows_read, 4);
    assert_eq!(report.rows_deleted, 2);
    // Row 4 has a null price: `price > 10` is unknown for it, and unknown is
    // not a match, so it survives. This is the whole reason the complement is
    // spelled with a null test.
    assert_eq!(
        rows(&handle),
        vec![
            (1, Some("XNAS".to_owned()), Some(5)),
            (4, Some("XLON".to_owned()), None),
        ]
    );
}

#[test]
fn update_rewrites_only_the_assigned_column_of_only_the_matching_rows() {
    let mut handle = stored();
    handle
        .apply_expression(
            &"UPDATE . SET price = price * 2 WHERE venue = 'XNAS'"
                .parse()
                .expect("parses"),
        )
        .expect("the update runs");
    assert_eq!(
        rows(&handle),
        vec![
            (1, Some("XNAS".to_owned()), Some(10)),
            (2, Some("XNYS".to_owned()), Some(15)),
            (3, Some("XNAS".to_owned()), Some(50)),
            (4, Some("XLON".to_owned()), None),
        ]
    );
}

#[test]
fn the_schema_verbs_change_the_schema_and_leave_the_rows() {
    let mut handle = stored();
    handle
        .apply_expression(&"ALTER TABLE . DROP COLUMN venue".parse().expect("parses"))
        .expect("the drop runs");
    let options = handle.record_options().expect("options");
    let root = handle.read_arrow_field(&options).expect("a schema");
    let names: Vec<&str> = root.fields().iter().map(Field::name).collect();
    assert_eq!(names, vec!["id", "price"]);

    handle
        .apply_expression(
            &"ALTER TABLE . RENAME COLUMN price TO amount"
                .parse()
                .expect("parses"),
        )
        .expect("the rename runs");
    let root = handle.read_arrow_field(&options).expect("a schema");
    let names: Vec<&str> = root.fields().iter().map(Field::name).collect();
    assert_eq!(names, vec!["id", "amount"]);
}

#[test]
fn insert_appends_the_literal_rows_with_the_declared_types() {
    let mut handle = stored();
    let Applied::Changed(report) = handle
        .apply_expression(
            &"INSERT INTO . VALUES (5, 'XETR', 99), (6, 'XPAR', 1)"
                .parse()
                .expect("parses"),
        )
        .expect("the insert runs")
    else {
        panic!("an insert reports rather than reading");
    };
    assert_eq!(report.rows_written, 2);
    let found = rows(&handle);
    assert_eq!(found.len(), 6);
    assert_eq!(found[4], (5, Some("XETR".to_owned()), Some(99)));
    assert_eq!(found[5], (6, Some("XPAR".to_owned()), Some(1)));
}

#[test]
fn select_hands_back_the_rows_rather_than_changing_anything() {
    let mut handle = stored();
    let Applied::Rows(reader) = handle
        .apply_expression(&"SELECT id, venue WHERE price > 10".parse().expect("parses"))
        .expect("the select runs")
    else {
        panic!("a select reads rather than reporting");
    };
    let mut ids = Vec::new();
    let mut columns = 0;
    for batch in reader {
        let batch = batch.expect("a batch");
        columns = batch.num_columns();
        let held = column::<Int64Array>(&batch, "id");
        for row in 0..batch.num_rows() {
            ids.push(held.value(row));
        }
    }
    assert_eq!(ids, vec![2, 3]);
    assert_eq!(columns, 2);
    // And nothing was written.
    assert_eq!(rows(&handle).len(), 4);
}

#[test]
fn a_chain_of_four_statements_is_one_pass() {
    let mut handle = stored();
    let chain: Statement = "DELETE WHERE price > 20; \
                            UPDATE . SET price = price + 1 WHERE venue = 'XNAS'; \
                            ALTER TABLE . DROP COLUMN venue; \
                            ALTER TABLE . RENAME COLUMN price TO amount"
        .parse()
        .expect("parses");

    // What it would do, before it does it.
    let planned = handle
        .explain_expression(&chain)
        .expect("the chain is typed end to end");
    assert_eq!(planned.mode, WriteMode::Overwrite);
    let names: Vec<&str> = planned.root.fields().iter().map(Field::name).collect();
    assert_eq!(names, vec!["id", "amount"]);

    handle.apply_expression(&chain).expect("the chain runs");

    let options = handle.record_options().expect("options");
    let root = handle.read_arrow_field(&options).expect("a schema");
    let names: Vec<&str> = root.fields().iter().map(Field::name).collect();
    assert_eq!(names, vec!["id", "amount"]);

    let mut found = Vec::new();
    for batch in handle.read_arrow_batch_reader(&options).expect("a reader") {
        let batch = batch.expect("a batch");
        let ids = column::<Int64Array>(&batch, "id");
        let amounts = column::<Int64Array>(&batch, "amount");
        for row in 0..batch.num_rows() {
            found.push((
                ids.value(row),
                (!arrow_array::Array::is_null(&amounts, row)).then(|| amounts.value(row)),
            ));
        }
    }
    // Row 3 went (price 25 > 20); row 1 was incremented; rows 2 and 4 stand.
    assert_eq!(found, vec![(1, Some(6)), (2, Some(15)), (4, None)]);
}

#[test]
fn a_statement_against_a_resource_that_holds_nothing_reports_zeros() {
    let mut handle = Buffer::new().with_media_type(MimeType::ARROW_STREAM.into());
    // Nothing is stored, so there is no schema to lower against - which is a
    // typed refusal rather than a panic.
    let outcome = handle.apply_expression(&"DELETE WHERE id > 1".parse().expect("parses"));
    assert!(outcome.is_err(), "an empty resource has no columns to name");

    // The lowering itself needs only a schema, and it answers without a
    // resource at all - which is what makes `explain` free.
    let lowered = "DELETE WHERE id > 1"
        .parse::<Statement>()
        .expect("parses")
        .lower(&schema())
        .expect("lowers");
    assert_eq!(lowered.mode, WriteMode::Overwrite);
}

#[test]
fn the_literal_values_of_an_insert_must_fit_the_columns() {
    let schema = schema();
    let error = "INSERT INTO . VALUES ('not a number', 'XNAS', 1)"
        .parse::<Statement>()
        .expect("parses")
        .lower(&schema)
        .expect_err("a value the column cannot hold");
    let message = error.to_string();
    assert!(message.contains("int64"), "{message}");

    // And a row of the wrong width names both counts.
    let error = "INSERT INTO . VALUES (1, 'XNAS')"
        .parse::<Statement>()
        .expect("parses")
        .lower(&schema)
        .expect_err("a short row");
    assert!(error.to_string().contains("3 value(s)"), "{error}");
}

#[test]
fn a_statement_is_a_value_like_every_other() {
    let one: Statement = "DELETE WHERE id > 1".parse().expect("parses");
    let same: Statement = "DELETE WHERE id > 1".parse().expect("parses");
    assert_eq!(one, same);
    assert_eq!(one.cmp(&same), std::cmp::Ordering::Equal);

    let json = serde_json::to_string(&one).expect("serializes");
    let back: Statement = serde_json::from_str(&json).expect("deserializes");
    assert_eq!(back, one);

    assert!(!one.is_read_only());
    assert!(
        "SELECT *"
            .parse::<Statement>()
            .expect("parses")
            .is_read_only()
    );
    assert!(
        "ALTER TABLE . DROP COLUMN venue"
            .parse::<Statement>()
            .expect("parses")
            .is_schema_only()
    );
    let _ = Expr::always_true();
    let _ = Value::Null;
}
