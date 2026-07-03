//! Integration tests for the `union` data type (the logical `optional` type built
//! on it is covered in `tests/optional.rs`).

use yggdryl_dtype::{
    arrow_schema, DataError, DataType, DataTypeId, Int64Type, Nested, Union, UnionType,
};

#[test]
fn union_describes_itself() {
    let union = UnionType::optional(&Int64Type);
    assert_eq!(union.name(), "union");
    assert_eq!(union.arrow_format(), "+us:0,1");
    assert_eq!(union.byte_width(), None);
    assert_eq!(union.child_count(), 2);
    assert_eq!(union.mode(), arrow_schema::UnionMode::Sparse);
    assert_eq!(UnionType::ID, DataTypeId::Union);
    assert_eq!(UnionType::ID.name(), union.name());
    assert_eq!(UnionType::ID.arrow_format(), None); // parameterized at the id level

    // The two variants carry the declared type ids and types.
    let children: Vec<_> = union
        .fields()
        .iter()
        .map(|(id, field)| (id, field.data_type().clone()))
        .collect();
    assert_eq!(
        children,
        vec![
            (UnionType::NULL_TYPE_ID, arrow_schema::DataType::Null),
            (UnionType::VALUE_TYPE_ID, arrow_schema::DataType::Int64),
        ]
    );
}

#[test]
fn union_arrow_round_trips_losslessly() {
    // A dense union with child metadata: from_arrow(to_arrow) preserves everything,
    // including what the fixed-width families do not model.
    let fields = arrow_schema::UnionFields::try_new(
        [2, 7],
        [
            arrow_schema::Field::new("a", arrow_schema::DataType::Int64, false),
            arrow_schema::Field::new("b", arrow_schema::DataType::Utf8, true).with_metadata(
                std::collections::HashMap::from([("app:origin".to_string(), "b".to_string())]),
            ),
        ],
    )
    .unwrap();
    let union = UnionType::new(fields, arrow_schema::UnionMode::Dense);
    assert_eq!(union.arrow_format(), "+ud:2,7");
    let arrow = union.to_arrow();
    assert_eq!(UnionType::from_arrow(&arrow).unwrap(), union);

    assert!(matches!(
        UnionType::from_arrow(&arrow_schema::DataType::Int64),
        Err(DataError::IncompatibleArrowType { .. })
    ));
}

#[test]
fn union_is_object_safe_and_send_sync() {
    fn assert_send_sync<T: Send + Sync>() {}
    assert_send_sync::<UnionType>();

    // A heterogeneous schema mixes primitives and unions through the vtable.
    let types: Vec<Box<dyn DataType>> = vec![
        Box::new(Int64Type),
        Box::new(UnionType::optional(&Int64Type)),
    ];
    assert_eq!(types[1].name(), "union");
    assert_eq!(types[1].arrow_format(), "+us:0,1");
    assert!(matches!(
        types[1].to_arrow(),
        arrow_schema::DataType::Union(..)
    ));
}
