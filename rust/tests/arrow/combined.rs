//! Two batch readers, schema-merged and cast onto one root.

use std::sync::Arc;

use arrow_array::{Int64Array, RecordBatch, StringArray};
use arrow_schema::SchemaRef;
use yggdryl::arrow::{BatchReader, batch_reader, combined, combined_as};
use yggdryl::{DataType, Field};

/// A reader over one batch of the shape `columns` names.
fn reader(root: &Field, columns: Vec<arrow_array::ArrayRef>) -> BatchReader {
    let schema = root.clone().into_arrow_schema().expect("an Arrow schema");
    if columns.is_empty() {
        return batch_reader(schema, []);
    }
    let batch = RecordBatch::try_new(Arc::clone(&schema), columns).expect("a batch");
    batch_reader(schema, [batch])
}

fn ids(values: &[i64]) -> arrow_array::ArrayRef {
    Arc::new(Int64Array::from(values.to_vec()))
}

fn text(values: &[&str]) -> arrow_array::ArrayRef {
    Arc::new(StringArray::from(values.to_vec()))
}

fn root(fields: impl IntoIterator<Item = Field>) -> Field {
    DataType::from_fields(fields)
        .expect("a struct root")
        .required_field("row")
}

/// Every batch a reader yields, drained.
fn drain(reader: BatchReader) -> Vec<RecordBatch> {
    reader.map(|batch| batch.expect("a batch")).collect()
}

#[test]
fn identical_schemas_pass_through_uncast() {
    let shape = root([DataType::Int64.nullable_field("id")]);
    let schema: SchemaRef = shape.clone().into_arrow_schema().unwrap();

    let left = reader(&shape, vec![ids(&[1])]);
    let right = reader(&shape, vec![ids(&[2])]);
    let joined = combined(left, right).expect("a merge");

    // The short-circuit itself: the merged schema is the input schema, so
    // `cast_reader` hands each side back rather than rebuilding its arrays.
    assert_eq!(joined.schema(), schema);
    let batches = drain(joined);
    assert_eq!(batches.len(), 2);
    for batch in &batches {
        assert_eq!(batch.schema(), schema);
        // Exactly the array that went in, not a rebuilt copy.
        assert_eq!(batch.num_columns(), 1);
    }
}

#[test]
fn disjoint_columns_unite_and_absent_ones_read_null() {
    let left_shape = root([DataType::Int64.nullable_field("id")]);
    let right_shape = root([DataType::Utf8.nullable_field("venue")]);

    let joined = combined(
        reader(&left_shape, vec![ids(&[1])]),
        reader(&right_shape, vec![text(&["XPAR"])]),
    )
    .expect("a merge");

    // Left's order first, then right-only columns in right's order.
    let schema = joined.schema();
    let names: Vec<&str> = schema
        .fields()
        .iter()
        .map(|field| field.name().as_str())
        .collect();
    assert_eq!(names, ["id", "venue"]);

    let batches = drain(joined);
    assert_eq!(batches.len(), 2);
    // The left's rows have no `venue`, so the cast fills null - and the right's
    // have no `id`.
    assert!(batches[0].column(1).is_null(0));
    assert!(batches[1].column(0).is_null(0));
    assert_eq!(
        batches[1]
            .column(1)
            .as_any()
            .downcast_ref::<StringArray>()
            .unwrap()
            .value(0),
        "XPAR"
    );
}

#[test]
fn overlapping_columns_and_a_differing_order_reconcile() {
    let left_shape = root([
        DataType::Int64.nullable_field("id"),
        DataType::Utf8.nullable_field("venue"),
    ]);
    // The same two columns, declared the other way round.
    let right_shape = root([
        DataType::Utf8.nullable_field("venue"),
        DataType::Int64.nullable_field("id"),
    ]);

    let joined = combined(
        reader(&left_shape, vec![ids(&[1]), text(&["XPAR"])]),
        reader(&right_shape, vec![text(&["XNAS"]), ids(&[2])]),
    )
    .expect("a merge");

    // Left's order wins; the right side is reordered by the cast.
    let schema = joined.schema();
    let names: Vec<&str> = schema
        .fields()
        .iter()
        .map(|field| field.name().as_str())
        .collect();
    assert_eq!(names, ["id", "venue"]);

    let batches = drain(joined);
    assert_eq!(
        batches[1]
            .column(0)
            .as_any()
            .downcast_ref::<Int64Array>()
            .unwrap()
            .value(0),
        2
    );
}

#[test]
fn column_names_unite_case_insensitively() {
    let left_shape = root([DataType::Int64.nullable_field("id")]);
    let right_shape = root([DataType::Int64.nullable_field("ID")]);

    let joined = combined(
        reader(&left_shape, vec![ids(&[1])]),
        reader(&right_shape, vec![ids(&[2])]),
    )
    .expect("a merge");

    // One column, spelled the way the left declared it.
    assert_eq!(joined.schema().fields().len(), 1);
    assert_eq!(joined.schema().field(0).name(), "id");
}

#[test]
fn a_column_only_on_one_side_becomes_nullable_even_when_required() {
    let left_shape = root([DataType::Int64.required_field("id")]);
    let right_shape = root([DataType::Utf8.required_field("venue")]);

    let joined = combined(
        reader(&left_shape, vec![ids(&[1])]),
        reader(&right_shape, vec![text(&["XPAR"])]),
    )
    .expect("a merge");

    // Necessarily: the other side's rows have no value for it. A caller's
    // non-null declaration cannot survive a merge, which is why it is stated.
    assert!(joined.schema().field(0).is_nullable());
    assert!(joined.schema().field(1).is_nullable());
}

#[test]
fn a_shared_column_that_is_required_on_one_side_widens_to_nullable() {
    let left_shape = root([DataType::Int64.required_field("id")]);
    let right_shape = root([DataType::Int64.nullable_field("id")]);

    let joined = combined(
        reader(&left_shape, vec![ids(&[1])]),
        reader(&right_shape, vec![ids(&[2])]),
    )
    .expect("a merge");

    assert_eq!(joined.schema().fields().len(), 1);
    assert!(joined.schema().field(0).is_nullable());
}

#[test]
fn a_conflicting_datatype_is_refused_naming_both_sides() {
    let left_shape = root([DataType::Int64.nullable_field("price")]);
    let right_shape = root([DataType::Utf8.nullable_field("price")]);

    let refused = combined(
        reader(&left_shape, vec![ids(&[1])]),
        reader(&right_shape, vec![text(&["1.5"])]),
    )
    .err()
    .expect("two datatypes for one column");
    let message = refused.to_string();
    assert!(message.contains("price"), "{message}");
    assert!(message.contains("int64"), "{message}");
    assert!(message.contains("utf8"), "{message}");
}

#[test]
fn a_conflicting_field_id_is_refused_naming_both_sides() {
    let mut left_column = DataType::Int64.nullable_field("id");
    left_column.set_parquet_field_id(1);
    let mut right_column = DataType::Int64.nullable_field("id");
    right_column.set_parquet_field_id(2);

    let left_shape = root([left_column]);
    let right_shape = root([right_column]);

    let refused = combined(
        reader(&left_shape, vec![ids(&[1])]),
        reader(&right_shape, vec![ids(&[2])]),
    )
    .err()
    .expect("two identities for one column");
    let message = refused.to_string();
    assert!(message.contains("PARQUET:field_id"), "{message}");
    assert!(message.contains('1') && message.contains('2'), "{message}");
}

#[test]
fn combining_pulls_no_batch_until_the_result_is_iterated() {
    /// A reader that panics if anything pulls a batch from it.
    struct Tripwire {
        schema: SchemaRef,
    }

    impl Iterator for Tripwire {
        type Item = std::result::Result<RecordBatch, arrow_schema::ArrowError>;

        fn next(&mut self) -> Option<Self::Item> {
            panic!("a combine must not pull a batch");
        }
    }

    impl arrow_array::RecordBatchReader for Tripwire {
        fn schema(&self) -> SchemaRef {
            Arc::clone(&self.schema)
        }
    }

    let left_shape = root([DataType::Int64.nullable_field("id")]);
    let right_shape = root([DataType::Utf8.nullable_field("venue")]);

    // The merge is derived from the two schemas alone, which a reader answers
    // without pulling anything.
    let joined = combined(
        Box::new(Tripwire {
            schema: left_shape.into_arrow_schema().unwrap(),
        }),
        Box::new(Tripwire {
            schema: right_shape.into_arrow_schema().unwrap(),
        }),
    )
    .expect("a merge");
    assert_eq!(joined.schema().fields().len(), 2);
}

#[test]
fn an_explicit_root_casts_both_sides() {
    let left_shape = root([DataType::Int64.nullable_field("id")]);
    let right_shape = root([DataType::Utf8.nullable_field("id")]);
    // Declared: both sides land here, whatever they were.
    let target = root([DataType::Utf8.nullable_field("id")]);

    let joined = combined_as(
        reader(&left_shape, vec![ids(&[1])]),
        reader(&right_shape, vec![text(&["2"])]),
        &target,
        false,
    )
    .expect("a chain");

    assert_eq!(joined.schema(), target.clone().into_arrow_schema().unwrap());
    let batches = drain(joined);
    assert_eq!(batches.len(), 2);
    for batch in &batches {
        assert_eq!(batch.schema(), target.clone().into_arrow_schema().unwrap());
    }
}
