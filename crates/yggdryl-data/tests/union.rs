//! Integration tests for the `null` and `union` families and the [`OptionalScalar`]
//! built on the null-or-value union.

use yggdryl_data::arrow_array::Array;
use yggdryl_data::{
    arrow_array, arrow_schema, DataError, DataTypeId, Int64, Int64Scalar, Nested, Null, NullField,
    NullScalar, OptionalScalar, RawDataType, RawField, RawScalar, UInt8, UInt8Scalar, Union,
    UnionField,
};

type OptionalInt64 = OptionalScalar<Int64, Int64Scalar>;

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
fn optional_scalar_holds_a_value_or_the_null_variant() {
    let answer = OptionalInt64::new(Int64Scalar::new(42));
    assert!(!answer.is_null());
    assert_eq!(answer.value(), Some(&42));
    assert_eq!(answer.scalar(), Some(&Int64Scalar::new(42)));
    assert_eq!(answer.data_type(), &Union::optional(&Int64));

    let missing = OptionalInt64::null();
    assert!(missing.is_null());
    assert_eq!(missing.value(), None);
    assert_eq!(missing.scalar(), None);
    assert_eq!(OptionalInt64::default(), missing);

    // Built from the inner scalar, or an Option of it.
    assert_eq!(OptionalInt64::from(Int64Scalar::new(42)), answer);
    assert_eq!(OptionalInt64::from(Some(Int64Scalar::new(42))), answer);
    assert_eq!(OptionalInt64::from(None::<Int64Scalar>), missing);

    // A null *inner* scalar is still null: the two representations agree.
    let inner_null = OptionalInt64::new(Int64Scalar::null());
    assert!(inner_null.is_null());
    assert_eq!(inner_null.value(), None);
}

#[test]
fn optional_scalar_redirects_access_to_the_inner_scalar() {
    let answer = OptionalInt64::new(Int64Scalar::new(42));
    assert_eq!(answer.as_i64(), Some(42));
    assert_eq!(answer.as_i8(), Some(42));
    assert_eq!(answer.as_f64(), Some(42.0));
    assert_eq!(answer.as_str(), None);
    assert_eq!(OptionalInt64::null().as_i64(), None);

    // Any inner scalar type works the same way.
    let flag = OptionalScalar::new(UInt8Scalar::new(7));
    assert_eq!(flag.as_u8(), Some(7));
    assert_eq!(flag.data_type(), &Union::optional(&UInt8));
}

#[test]
fn optional_scalar_arrow_round_trips_both_variants() {
    // Value variant: a one-element sparse union selecting the value child.
    let answer = OptionalInt64::new(Int64Scalar::new(42));
    let arrow = answer.to_arrow();
    assert_eq!(arrow.len(), 1);
    assert_eq!(arrow.data_type(), &Union::optional(&Int64).to_arrow());
    let union_array = arrow
        .as_any()
        .downcast_ref::<arrow_array::UnionArray>()
        .unwrap();
    assert_eq!(union_array.type_id(0), Union::VALUE_TYPE_ID);
    assert_eq!(OptionalInt64::from_arrow(arrow.as_ref()).unwrap(), answer);

    // Null variant: the type id selects the null child.
    let missing = OptionalInt64::null();
    let arrow = missing.to_arrow();
    let union_array = arrow
        .as_any()
        .downcast_ref::<arrow_array::UnionArray>()
        .unwrap();
    assert_eq!(union_array.type_id(0), Union::NULL_TYPE_ID);
    assert_eq!(OptionalInt64::from_arrow(arrow.as_ref()).unwrap(), missing);

    // A null inner scalar normalizes to the null variant on the way out.
    let inner_null = OptionalInt64::new(Int64Scalar::null());
    let arrow = inner_null.to_arrow();
    let union_array = arrow
        .as_any()
        .downcast_ref::<arrow_array::UnionArray>()
        .unwrap();
    assert_eq!(union_array.type_id(0), Union::NULL_TYPE_ID);
    assert!(OptionalInt64::from_arrow(arrow.as_ref()).unwrap().is_null());
}

#[test]
fn optional_scalar_from_arrow_rejects_other_shapes() {
    // A non-union array of the right length.
    let plain = arrow_array::Int64Array::from_iter_values([42]);
    assert!(matches!(
        OptionalInt64::from_arrow(&plain),
        Err(DataError::IncompatibleArrowType { .. })
    ));

    // The right union layout but for a different value type.
    let other = OptionalScalar::new(UInt8Scalar::new(7)).to_arrow();
    assert!(matches!(
        OptionalInt64::from_arrow(other.as_ref()),
        Err(DataError::IncompatibleArrowType { .. })
    ));

    // The right layout at the wrong length.
    let fields = Union::optional(&Int64).fields().clone();
    let two = arrow_array::UnionArray::try_new(
        fields,
        vec![Union::VALUE_TYPE_ID, Union::NULL_TYPE_ID].into(),
        None,
        vec![
            std::sync::Arc::new(arrow_array::NullArray::new(2)),
            std::sync::Arc::new(arrow_array::Int64Array::from_iter_values([1, 2])),
        ],
    )
    .unwrap();
    assert!(matches!(
        OptionalInt64::from_arrow(&two),
        Err(DataError::InvalidScalarLength { got: 2 })
    ));
}

#[test]
fn union_is_object_safe_and_send_sync() {
    fn assert_send_sync<T: Send + Sync>() {}
    assert_send_sync::<Union>();
    assert_send_sync::<UnionField>();
    assert_send_sync::<OptionalInt64>();
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
