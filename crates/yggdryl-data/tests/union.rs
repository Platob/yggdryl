//! Integration tests for the `null` and `union` families (the logical `optional`
//! type built on them is covered in `tests/optional.rs`).

use yggdryl_data::arrow_array::Array;
use yggdryl_data::{
    arrow_array, arrow_schema, DataError, DataTypeId, Int64, Nested, Null, NullField, NullScalar,
    RawDataType, RawField, RawScalar, Union, UnionField,
};

#[test]
fn null_describes_itself_and_round_trips() {
    assert_eq!(Null.name(), "null");
    assert_eq!(Null.arrow_format(), "n");
    assert_eq!((Null.byte_width(), Null.bit_width()), (None, None));
    assert_eq!(Null::ID, DataTypeId::Null);
    assert_eq!(Null::ID.name(), Null.name());
    assert_eq!(Null::ID.arrow_format(), Some("n"));

    assert_eq!(Null.to_arrow(), arrow_schema::DataType::Null);
    assert_eq!(Null::from_arrow(&Null.to_arrow()).unwrap(), Null);
    assert!(Null::from_arrow(&arrow_schema::DataType::Int64).is_err());

    let gap = NullField::new("gap", true);
    assert_eq!(NullField::from_arrow(&gap.to_arrow()).unwrap(), gap);

    let nothing = NullScalar::new();
    assert!(nothing.is_null());
    assert_eq!(nothing.value(), None);
    let arrow = nothing.to_arrow();
    assert_eq!(arrow.len(), 1);
    assert_eq!(arrow.data_type(), &arrow_schema::DataType::Null);
    assert_eq!(NullScalar::from_arrow(arrow.as_ref()).unwrap(), nothing);
    // Wrong length and wrong type are refused.
    assert!(matches!(
        NullScalar::from_arrow(&arrow_array::NullArray::new(2)),
        Err(DataError::InvalidScalarLength { got: 2 })
    ));
    assert!(matches!(
        NullScalar::from_arrow(&arrow_array::Int64Array::from_iter_values([1])),
        Err(DataError::IncompatibleArrowType { .. })
    ));
}

#[test]
fn union_describes_itself() {
    let union = Union::optional(&Int64);
    assert_eq!(union.name(), "union");
    assert_eq!(union.arrow_format(), "+us:0,1");
    assert_eq!(union.byte_width(), None);
    assert_eq!(union.child_count(), 2);
    assert_eq!(union.mode(), arrow_schema::UnionMode::Sparse);
    assert_eq!(Union::ID, DataTypeId::Union);
    assert_eq!(Union::ID.name(), union.name());
    assert_eq!(Union::ID.arrow_format(), None); // parameterized at the id level

    // The two variants carry the declared type ids and types.
    let children: Vec<_> = union
        .fields()
        .iter()
        .map(|(id, field)| (id, field.data_type().clone()))
        .collect();
    assert_eq!(
        children,
        vec![
            (Union::NULL_TYPE_ID, arrow_schema::DataType::Null),
            (Union::VALUE_TYPE_ID, arrow_schema::DataType::Int64),
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
    let union = Union::new(fields, arrow_schema::UnionMode::Dense);
    assert_eq!(union.arrow_format(), "+ud:2,7");
    let arrow = union.to_arrow();
    assert_eq!(Union::from_arrow(&arrow).unwrap(), union);

    assert!(matches!(
        Union::from_arrow(&arrow_schema::DataType::Int64),
        Err(DataError::IncompatibleArrowType { .. })
    ));
}

#[test]
fn union_field_round_trips_and_applies_the_metadata_policy() {
    let field = UnionField::new("value", Union::optional(&Int64), true);
    let arrow = field.to_arrow();
    assert_eq!(arrow.name(), "value");
    assert!(arrow.is_nullable());
    assert_eq!(UnionField::from_arrow(&arrow).unwrap(), field);

    // An extension-typed field is a different logical type.
    let extension = field
        .to_arrow()
        .with_metadata(std::collections::HashMap::from([(
            "ARROW:extension:name".to_string(),
            "arrow.opaque".to_string(),
        )]));
    assert!(matches!(
        UnionField::from_arrow(&extension),
        Err(DataError::IncompatibleArrowType { .. })
    ));

    // A field of a non-union type is refused.
    let wrong = arrow_schema::Field::new("value", arrow_schema::DataType::Int64, true);
    assert!(matches!(
        UnionField::from_arrow(&wrong),
        Err(DataError::IncompatibleArrowType { .. })
    ));
}

#[test]
fn union_is_object_safe_and_send_sync() {
    fn assert_send_sync<T: Send + Sync>() {}
    assert_send_sync::<Union>();
    assert_send_sync::<UnionField>();
    assert_send_sync::<Null>();
    assert_send_sync::<NullScalar>();

    // A heterogeneous schema mixes primitives and unions through the vtable.
    let types: Vec<Box<dyn RawDataType>> = vec![Box::new(Int64), Box::new(Union::optional(&Int64))];
    assert_eq!(types[1].name(), "union");
    assert_eq!(types[1].arrow_format(), "+us:0,1");
    assert!(matches!(
        types[1].to_arrow(),
        arrow_schema::DataType::Union(..)
    ));
}
