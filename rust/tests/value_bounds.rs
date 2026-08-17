//! Adversarial physical-materialization budget tests.

use arrow_array::Array;
use yggdryl::arrow::{ArrowScalar, schema_from_field};
use yggdryl::{DataType, Field, UnionMode, Value};

fn union_value(type_id: i8, payload: Value) -> Value {
    Value::from_sequence([Value::from(type_id), payload])
}

fn fixed_list_field(name: &str, length: i32, nullable: bool) -> Field {
    Field::new(
        name,
        DataType::fixed_size_list(Field::new("item", DataType::Int32, false), length).unwrap(),
        nullable,
    )
}

#[test]
fn nullable_fixed_list_rejects_hidden_slot_expansion_before_allocation() {
    let field = Field::new(
        "items",
        DataType::fixed_size_list(Field::new("item", DataType::Int32, false), 1_000_001).unwrap(),
        true,
    );
    let error = ArrowScalar::from_value(field, Value::Null).unwrap_err();
    let message = error.to_string();
    assert!(message.contains("expanded slots"), "{message}");
    assert!(message.contains("expected at most 1000000"), "{message}");
    assert!(message.contains("got 1000001"), "{message}");
}

#[test]
fn sparse_union_joins_selected_and_inactive_children_into_one_budget() {
    let fields = [
        (1, fixed_list_field("selected", 600_000, true)),
        (2, fixed_list_field("inactive", 600_000, true)),
    ];
    let sparse = DataType::union(fields, UnionMode::Sparse).unwrap();
    let error = ArrowScalar::from_value(
        Field::new("choice", sparse, true),
        union_value(1, Value::Null),
    )
    .unwrap_err();
    let message = error.to_string();
    assert!(message.contains("expanded slots"), "{message}");
    assert!(message.contains("expected at most 1000000"), "{message}");
    assert!(message.contains("got 1200002"), "{message}");
}

#[test]
fn dense_union_does_not_visit_an_inactive_oversized_default() {
    let oversized = DataType::fixed_size_binary(64 * 1024 * 1024 + 1).unwrap();
    let dense = DataType::union(
        [
            (0, Field::new("selected", DataType::Int32, false)),
            (1, Field::new("inactive", oversized, false)),
        ],
        UnionMode::Dense,
    )
    .unwrap();
    let expected = union_value(0, Value::from(11_i32));
    let scalar =
        ArrowScalar::from_value(Field::new("choice", dense, false), expected.clone()).unwrap();
    assert_eq!(scalar.to_value().unwrap(), expected);
}

#[test]
fn dense_union_remains_lazy_below_a_generated_struct_slot() {
    let dense = DataType::union(
        [
            (0, Field::new("selected", DataType::Int32, false)),
            (
                1,
                Field::new(
                    "inactive",
                    DataType::fixed_size_binary(64 * 1024 * 1024 + 1).unwrap(),
                    false,
                ),
            ),
        ],
        UnionMode::Dense,
    )
    .unwrap();
    let structure = DataType::from_fields([Field::new("choice", dense, false)]).unwrap();
    let scalar = ArrowScalar::from_value(Field::new("row", structure, true), Value::Null).unwrap();
    assert_eq!(scalar.to_value().unwrap(), Value::Null);
}

#[test]
fn nullable_struct_rejects_aggregate_fixed_physical_bytes() {
    let width = 40 * 1024 * 1024;
    let structure = DataType::from_fields([
        Field::new("left", DataType::fixed_size_binary(width).unwrap(), false),
        Field::new("right", DataType::fixed_size_binary(width).unwrap(), false),
    ])
    .unwrap();
    let error =
        ArrowScalar::from_value(Field::new("wide", structure, true), Value::Null).unwrap_err();
    let message = error.to_string();
    assert!(message.contains("fixed bytes"), "{message}");
    assert!(message.contains("expected at most 67108864"), "{message}");
}

#[test]
fn nullable_struct_aggregates_selected_dense_union_payloads() {
    let member = || {
        DataType::union(
            [(
                4,
                Field::new(
                    "payload",
                    DataType::fixed_size_binary(40 * 1024 * 1024).unwrap(),
                    false,
                ),
            )],
            UnionMode::Dense,
        )
        .unwrap()
    };
    let structure = DataType::from_fields([
        Field::new("left", member(), false),
        Field::new("right", member(), false),
    ])
    .unwrap();
    let error =
        ArrowScalar::from_value(Field::new("wide", structure, true), Value::Null).unwrap_err();
    let message = error.to_string();
    assert!(message.contains("fixed bytes"), "{message}");
    assert!(message.contains("expected at most 67108864"), "{message}");
}

#[test]
fn dictionary_and_run_end_wrappers_join_the_hidden_byte_budget() {
    let width = 40 * 1024 * 1024;
    let dictionary =
        DataType::dictionary(DataType::Int8, DataType::fixed_size_binary(width).unwrap()).unwrap();
    let encoded = DataType::run_end_encoded(
        Field::new("run_ends", DataType::Int16, false),
        Field::new("values", DataType::fixed_size_binary(width).unwrap(), false),
    )
    .unwrap();
    let structure = DataType::from_fields([
        Field::new("dictionary", dictionary, false),
        Field::new("encoded", encoded, false),
    ])
    .unwrap();
    let error =
        ArrowScalar::from_value(Field::new("wide", structure, true), Value::Null).unwrap_err();
    let message = error.to_string();
    assert!(message.contains("fixed bytes"), "{message}");
    assert!(message.contains("expected at most 67108864"), "{message}");
}

#[test]
fn every_valid_nested_datatype_can_materialize_zero_rows_without_a_default() {
    let required_null_struct =
        || DataType::from_fields([Field::new("required_null", DataType::Null, false)]).unwrap();
    let wrappers = [
        DataType::list(Field::new("item", required_null_struct(), false)),
        DataType::dictionary(DataType::Int8, required_null_struct()).unwrap(),
        DataType::map_of(DataType::Int32, required_null_struct(), false).unwrap(),
        DataType::run_end_encoded(
            Field::new("run_ends", DataType::Int16, false),
            Field::new("values", required_null_struct(), false),
        )
        .unwrap(),
        DataType::union(
            [(0, Field::new("member", required_null_struct(), false))],
            UnionMode::Dense,
        )
        .unwrap(),
        DataType::union(
            [(0, Field::new("member", required_null_struct(), false))],
            UnionMode::Sparse,
        )
        .unwrap(),
    ];

    for (index, data_type) in wrappers.into_iter().enumerate() {
        let root = Field::new(
            "Root",
            DataType::from_fields([Field::new("value", data_type, false)]).unwrap(),
            false,
        );
        let schema = schema_from_field(&root)
            .unwrap_or_else(|error| panic!("empty wrapper {index} failed: {error}"));
        let array = arrow_array::RecordBatch::new_empty(schema);
        assert_eq!(array.num_rows(), 0);
        assert_eq!(array.column(0).len(), 0);
    }
}

#[test]
fn masked_hidden_slots_do_not_require_a_logical_default() {
    let impossible =
        || DataType::from_fields([Field::new("nothing", DataType::Null, false)]).unwrap();

    let structure = DataType::from_fields([Field::new("inner", impossible(), false)]).unwrap();
    let scalar =
        ArrowScalar::from_value(Field::new("outer", structure, true), Value::Null).unwrap();
    assert_eq!(scalar.to_value().unwrap(), Value::Null);

    let fixed = DataType::fixed_size_list(Field::new("item", impossible(), false), 2).unwrap();
    let scalar = ArrowScalar::from_value(Field::new("outer", fixed, true), Value::Null).unwrap();
    assert_eq!(scalar.to_value().unwrap(), Value::Null);

    let sparse = DataType::union(
        [
            (1, Field::new("selected", DataType::Int32, false)),
            (2, Field::new("hidden", impossible(), false)),
        ],
        UnionMode::Sparse,
    )
    .unwrap();
    let expected = union_value(1, Value::I64(7));
    let scalar =
        ArrowScalar::from_value(Field::new("choice", sparse, false), expected.clone()).unwrap();
    assert_eq!(scalar.to_value().unwrap(), expected);

    let encoded = DataType::run_end_encoded(
        Field::new("run_ends", DataType::Int16, false),
        Field::new("values", DataType::Null, false),
    )
    .unwrap();
    let nested = DataType::from_fields([Field::new("encoded", encoded, false)]).unwrap();
    let scalar = ArrowScalar::from_value(Field::new("outer", nested, true), Value::Null).unwrap();
    assert_eq!(scalar.to_value().unwrap(), Value::Null);
}
