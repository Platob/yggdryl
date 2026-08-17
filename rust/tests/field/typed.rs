use yggdryl::field::{self, FieldType, Int32Field, TimestampField, TypedField, TypedFieldRef};
use yggdryl::{DataType, Field, TimeUnit, Timezone};

pub(crate) fn assert_typed_marker<K: FieldType>(data_type: DataType) {
    let field = Field::new("value", data_type.clone(), false);
    let borrowed = field
        .try_as_typed::<K>()
        .expect("the marker must accept its datatype variant");
    assert_eq!(borrowed.data_type(), &data_type);
    let typed = field
        .try_into_typed::<K>()
        .expect("the owned marker must accept its datatype variant");
    assert_eq!(typed.into_field().data_type(), &data_type);
}

#[test]
fn typed_fields_are_zero_overhead_checked_and_lossless() {
    assert_eq!(
        std::mem::size_of::<TypedField<integer_marker::Int32>>(),
        std::mem::size_of::<Field>()
    );
    assert_eq!(
        std::mem::size_of::<TypedFieldRef<'_, integer_marker::Int32>>(),
        std::mem::size_of::<&Field>()
    );

    let mut typed = Int32Field::from_parts("id", false, [("source", "orders")]).unwrap();
    assert_eq!(typed.get_metadata("source"), Some("orders"));
    typed.insert_metadata("version", "1").unwrap();
    typed.set_nullable(true);
    typed.set_name("order_id");
    let generic = typed.into_field();
    assert_eq!(generic.name(), "order_id");
    assert!(generic.is_nullable());
    assert_eq!(generic.data_type(), &DataType::Int32);
    assert_eq!(generic.get_metadata("version"), Some("1"));

    assert!(generic.try_as_typed::<field::integer::Int64>().is_err());
    assert!(
        Field::new(
            "invalid",
            DataType::Decimal128 {
                precision: 0,
                scale: 0,
            },
            false,
        )
        .try_as_typed::<field::decimal::Decimal128>()
        .is_err()
    );
}

mod integer_marker {
    pub use yggdryl::field::integer::Int32;
}

#[test]
fn typed_datatype_replacement_is_transactional_and_same_variant_only() {
    let mut field = TimestampField::try_new(
        "created_at",
        DataType::Timestamp(TimeUnit::Millisecond, None),
        false,
    )
    .unwrap();
    let original = field.clone();
    assert!(field.set_data_type(DataType::Int64).is_err());
    assert_eq!(field, original);
    field
        .set_data_type(DataType::Timestamp(
            TimeUnit::Nanosecond,
            Some(Timezone::UTC),
        ))
        .unwrap();
    assert_eq!(
        field.data_type(),
        &DataType::Timestamp(TimeUnit::Nanosecond, Some(Timezone::UTC))
    );
}

#[test]
fn typed_serde_rejects_a_wrong_or_invalid_datatype() {
    let int64_json = Field::new("id", DataType::Int64, false).to_json().unwrap();
    assert!(serde_json::from_str::<Int32Field>(&int64_json).is_err());

    let invalid_decimal = r#"{"name":"amount","data_type":{"decimal128":{"precision":0,"scale":0}},"nullable":false,"metadata":{}}"#;
    assert!(
        serde_json::from_str::<TypedField<field::decimal::Decimal128>>(invalid_decimal).is_err()
    );
}
