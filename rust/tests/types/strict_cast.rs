//! What a strict cast refuses, and where in the schema it says so.
//!
//! The default policy repairs: a required column the source does not carry
//! becomes its canonical default, and so does a null inside one. That is the
//! right answer for a lake being filled and the wrong one for a contract being
//! enforced, so [`Nullability::Strict`] refuses both instead - by path, so a
//! caller reading the message knows which declaration was not met. These cases
//! pin the two refusals, the shapes that are *not* refused, and the fact that
//! `safe` still answers a different question.

use std::sync::Arc;

use arrow_array::builder::{Int32Builder, ListBuilder, MapBuilder, StringBuilder};
use arrow_array::{
    Array, ArrayRef, DictionaryArray, Int32Array, Int64Array, RecordBatch, StringArray, StructArray,
};
use arrow_array::types::Int16Type;
use arrow_schema::{
    DataType as ArrowDataType, Field as ArrowField, Fields, Schema, SchemaRef,
};
use yggdryl::{ArrowCast, ArrowCastOptions, ArrowCastPlan, DataType, Field, Nullability};

fn root(fields: impl IntoIterator<Item = Field>) -> Field {
    Field::new("row", DataType::from_fields(fields).unwrap(), false)
}

fn strict() -> ArrowCastOptions {
    ArrowCastOptions::new().with_nullability(Nullability::Strict)
}

fn schema(fields: Vec<ArrowField>) -> SchemaRef {
    Arc::new(Schema::new(fields))
}

fn refusal(target: &Field, batch: RecordBatch) -> String {
    target
        .cast_arrow_batch(batch, strict())
        .unwrap_err()
        .to_string()
}

#[test]
fn a_required_column_the_source_does_not_carry_is_refused_by_path() {
    let source = schema(vec![ArrowField::new("id", ArrowDataType::Int32, false)]);
    let batch =
        RecordBatch::try_new(Arc::clone(&source), vec![Arc::new(Int32Array::from(vec![1]))])
            .unwrap();
    let target = root([
        DataType::Int64.required_field("id"),
        DataType::Utf8.required_field("symbol"),
    ]);

    // The default policy is unchanged: the hole is filled, not reported.
    let filled = target
        .cast_arrow_batch(batch.clone(), ArrowCastOptions::new())
        .unwrap();
    assert_eq!(filled.column(1).null_count(), 0);
    assert_eq!(filled.num_columns(), 2);

    assert_eq!(
        refusal(&target, batch),
        "required Arrow field $.symbol is missing from the source"
    );

    // The schemas alone decide it, so compiling is where it fails - a reader
    // never pulls a batch to find out.
    let message = ArrowCastPlan::compile(&source, &target, strict())
        .unwrap_err()
        .to_string();
    assert_eq!(
        message,
        "required Arrow field $.symbol is missing from the source"
    );
}

#[test]
fn a_null_in_a_required_column_is_refused_with_its_count() {
    let source = schema(vec![
        ArrowField::new("id", ArrowDataType::Int64, false),
        ArrowField::new("symbol", ArrowDataType::Utf8, true),
    ]);
    let batch = RecordBatch::try_new(
        source,
        vec![
            Arc::new(Int64Array::from(vec![1, 2, 3])),
            Arc::new(StringArray::from(vec![Some("AAPL"), None, None])),
        ],
    )
    .unwrap();
    let target = root([
        DataType::Int64.required_field("id"),
        DataType::Utf8.required_field("symbol"),
    ]);

    let filled = target
        .cast_arrow_batch(batch.clone(), ArrowCastOptions::new())
        .unwrap();
    assert_eq!(filled.column(1).null_count(), 0);

    assert_eq!(
        refusal(&target, batch),
        "required Arrow field $.symbol holds 2 null values"
    );
}

#[test]
fn a_missing_nullable_column_stays_all_null_under_both_policies() {
    let source = schema(vec![ArrowField::new("id", ArrowDataType::Int64, false)]);
    let batch =
        RecordBatch::try_new(source, vec![Arc::new(Int64Array::from(vec![1, 2]))]).unwrap();
    let target = root([
        DataType::Int64.required_field("id"),
        DataType::Utf8.nullable_field("symbol"),
    ]);

    for options in [ArrowCastOptions::new(), strict()] {
        let cast = target.cast_arrow_batch(batch.clone(), options).unwrap();
        assert_eq!(cast.num_columns(), 2);
        assert_eq!(cast.column(1).null_count(), 2);
    }
}

#[test]
fn an_undeclared_source_column_stays_dropped_under_both_policies() {
    let source = schema(vec![
        ArrowField::new("id", ArrowDataType::Int64, false),
        ArrowField::new("unused", ArrowDataType::Int32, true),
    ]);
    let batch = RecordBatch::try_new(
        source,
        vec![
            Arc::new(Int64Array::from(vec![1])),
            Arc::new(Int32Array::from(vec![Some(7)])),
        ],
    )
    .unwrap();
    let target = root([DataType::Int64.required_field("id")]);

    for options in [ArrowCastOptions::new(), strict()] {
        let cast = target.cast_arrow_batch(batch.clone(), options).unwrap();
        assert_eq!(cast.schema().fields().len(), 1);
        assert_eq!(cast.schema().field(0).name(), "id");
    }
}

#[test]
fn a_nested_struct_child_is_named_by_its_whole_path() {
    let address = ArrowDataType::Struct(Fields::from(vec![ArrowField::new(
        "city",
        ArrowDataType::Utf8,
        true,
    )]));
    let source = schema(vec![ArrowField::new("address", address.clone(), false)]);
    let batch = RecordBatch::try_new(
        source,
        vec![Arc::new(StructArray::new(
            Fields::from(vec![ArrowField::new("city", ArrowDataType::Utf8, true)]),
            vec![Arc::new(StringArray::from(vec!["Paris"])) as ArrayRef],
            None,
        ))],
    )
    .unwrap();

    // A required child the source struct does not carry at all.
    let missing = root([DataType::from_fields([
        DataType::Utf8.nullable_field("city"),
        DataType::Utf8.required_field("zip code"),
    ])
    .unwrap()
    .required_field("address")]);
    assert_eq!(
        refusal(&missing, batch.clone()),
        "required Arrow field $.address[\"zip code\"] is missing from the source"
    );

    // A required child the source carries, holding null.
    let null_source = schema(vec![ArrowField::new("address", address, false)]);
    let null_batch = RecordBatch::try_new(
        null_source,
        vec![Arc::new(StructArray::new(
            Fields::from(vec![ArrowField::new("city", ArrowDataType::Utf8, true)]),
            vec![Arc::new(StringArray::from(vec![None::<&str>])) as ArrayRef],
            None,
        ))],
    )
    .unwrap();
    let required_child = root([DataType::from_fields([DataType::Utf8.required_field("city")])
        .unwrap()
        .required_field("address")]);
    assert_eq!(
        refusal(&required_child, null_batch),
        "required Arrow field $.address.city holds 1 null values"
    );
}

#[test]
fn a_required_list_item_is_named_under_its_list() {
    let mut builder = ListBuilder::new(Int32Builder::new());
    builder.values().append_value(1);
    builder.values().append_null();
    builder.append(true);
    let values: ArrayRef = Arc::new(builder.finish());
    let source = schema(vec![ArrowField::new(
        "counts",
        values.data_type().clone(),
        true,
    )]);
    let batch = RecordBatch::try_new(source, vec![values]).unwrap();

    let target = root([
        DataType::List(Arc::new(DataType::Int32.required_field("item"))).nullable_field("counts"),
    ]);
    assert_eq!(
        refusal(&target, batch),
        "required Arrow field $.counts[] holds 1 null values"
    );
}

#[test]
fn a_required_map_value_is_named_under_its_entries() {
    let mut builder = MapBuilder::new(None, StringBuilder::new(), Int32Builder::new());
    builder.keys().append_value("a");
    builder.values().append_null();
    builder.append(true).unwrap();
    let map: ArrayRef = Arc::new(builder.finish());
    let source = schema(vec![ArrowField::new("tags", map.data_type().clone(), true)]);
    let batch = RecordBatch::try_new(source, vec![map]).unwrap();

    let entries = DataType::from_fields([
        DataType::Utf8.required_field("keys"),
        DataType::Int32.required_field("values"),
    ])
    .unwrap()
    .required_field("entries");
    let target = root([DataType::map(entries, false).unwrap().nullable_field("tags")]);
    assert_eq!(
        refusal(&target, batch),
        "required Arrow field $.tags.entries.values holds 1 null values"
    );
}

#[test]
fn a_dictionary_refuses_a_null_its_values_reach() {
    let dictionary: ArrayRef = Arc::new(DictionaryArray::<Int16Type>::from_iter([
        Some("alpha"),
        None,
    ]));
    let source = schema(vec![ArrowField::new(
        "label",
        dictionary.data_type().clone(),
        true,
    )]);
    let batch = RecordBatch::try_new(source, vec![dictionary]).unwrap();

    let target = root([DataType::dictionary(DataType::Int16, DataType::Utf8)
        .unwrap()
        .required_field("label")]);
    assert_eq!(
        refusal(&target, batch),
        "required Arrow field $.label holds 1 null values"
    );
}

#[test]
fn strictness_and_conversion_safety_answer_different_questions() {
    let source = schema(vec![ArrowField::new("id", ArrowDataType::Utf8, false)]);
    let batch = RecordBatch::try_new(
        source,
        vec![Arc::new(StringArray::from(vec!["1", "not a number"]))],
    )
    .unwrap();
    let target = root([DataType::Int64.required_field("id")]);

    // safe: the failed conversion becomes null, and strictness is what then
    // decides whether that null may stand in for a declared value.
    let repaired = target
        .cast_arrow_batch(batch.clone(), ArrowCastOptions::new())
        .unwrap();
    assert_eq!(repaired.column(0).null_count(), 0);
    assert_eq!(
        refusal(&target, batch.clone()),
        "required Arrow field $.id holds 1 null values"
    );

    // Unsafe: the conversion itself refuses, so strictness never sees a null.
    let unsafe_message = target
        .cast_arrow_batch(batch, strict().with_safe(false))
        .unwrap_err()
        .to_string();
    assert!(
        unsafe_message.contains("not a number"),
        "{unsafe_message}"
    );
}

#[test]
fn extension_and_schema_metadata_survive_a_strict_cast() {
    let source = schema(vec![ArrowField::new("id", ArrowDataType::Int32, false)]);
    let batch = RecordBatch::try_new(source, vec![Arc::new(Int32Array::from(vec![1]))]).unwrap();

    let mut identifier = DataType::Uuid.nullable_field("identifier");
    identifier.set_metadata([("owner", "trading")]).unwrap();
    let mut target = root([DataType::Int64.required_field("id"), identifier]);
    target.set_metadata([("source", "book")]).unwrap();

    let cast = target.cast_arrow_batch(batch, strict()).unwrap();
    let cast_schema = cast.schema();
    assert_eq!(
        cast_schema.metadata().get("source").map(String::as_str),
        Some("book")
    );
    let projected = cast_schema.field(1);
    assert_eq!(projected.metadata().get("owner").map(String::as_str), Some("trading"));
    assert_eq!(
        projected.extension_type_name(),
        Some("arrow.uuid"),
        "{projected:?}"
    );
}

#[test]
fn an_ambiguous_case_insensitive_name_is_refused_before_any_batch() {
    let source = schema(vec![
        ArrowField::new("id", ArrowDataType::Int64, false),
        ArrowField::new("ID", ArrowDataType::Int64, false),
    ]);
    let target = root([DataType::Int64.required_field("id")]);

    let message = ArrowCastPlan::compile(&source, &target, ArrowCastOptions::new())
        .unwrap_err()
        .to_string();
    assert!(message.contains("ambiguous"), "{message}");
}
