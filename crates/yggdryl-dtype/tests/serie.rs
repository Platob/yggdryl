//! Integration tests for the `serie` data type — the generic nested holder over one
//! value type.

use yggdryl_dtype::{
    arrow_schema, DataError, DataType, Int64Type, Nested, OptionalType, Serie, SerieType,
    TypedDataType, TypedNested, TypedSerie,
};

#[test]
fn list_describes_itself_and_round_trips() {
    let serie = SerieType::new(Int64Type);
    assert_eq!(serie.name(), "list");
    assert_eq!(serie.arrow_format(), "+l");
    assert_eq!(serie.byte_width(), None);
    assert_eq!(serie.child_count(), 1);
    assert_eq!(serie.value_type(), &Int64Type);

    assert_eq!(SerieType::from_arrow(&serie.to_arrow()).unwrap(), serie);
    assert!(matches!(
        SerieType::<Int64Type>::from_arrow(&arrow_schema::DataType::Int64),
        Err(DataError::IncompatibleArrowType { .. })
    ));
}

#[test]
fn list_codec_concatenates_elements() {
    let serie = SerieType::new(Int64Type);
    let bytes = serie.native_to_bytes(&vec![1, 2, 3]);
    assert_eq!(bytes.len(), 24);
    assert_eq!(serie.native_from_bytes(&bytes).unwrap(), vec![1, 2, 3]);
    assert_eq!(serie.native_from_bytes(&[]).unwrap(), Vec::<i64>::new());

    // A remainder is a length error naming the nearest whole-element length.
    assert!(matches!(
        serie.native_from_bytes(&[0; 9]),
        Err(DataError::InvalidByteLength {
            expected: 16,
            got: 9
        })
    ));

    // An optional element delegates its codec width to the value type, so the
    // round trip stays exact even though the *physical* width is indeterminate.
    let optional_list = SerieType::new(OptionalType::new(Int64Type));
    let bytes = optional_list.native_to_bytes(&vec![1, 2]);
    assert_eq!(optional_list.native_from_bytes(&bytes).unwrap(), vec![1, 2]);

    // A variable-width element cannot be split from bytes.
    let nested = SerieType::new(SerieType::new(Int64Type));
    assert!(matches!(
        nested.native_from_bytes(&[0; 8]),
        Err(DataError::IndeterminateElementWidth { .. })
    ));
}

#[test]
fn list_is_the_generic_nested_holder() {
    // The typed pair: Nested counts children, TypedNested<T> adds the native codec,
    // and TypedSerie pins the value type.
    fn raw_children<N: Nested>(nested: &N) -> usize {
        nested.child_count()
    }
    fn typed_default<T, N: TypedNested<T>>(nested: &N) -> T {
        nested.default_value()
    }
    fn serie_default<T, L: TypedSerie<T>>(serie: &L) -> Vec<T> {
        serie.default_value()
    }
    assert_eq!(raw_children(&SerieType::new(Int64Type)), 1);
    assert_eq!(
        typed_default::<Vec<i64>, _>(&SerieType::new(Int64Type)),
        Vec::<i64>::new()
    );
    assert_eq!(serie_default(&SerieType::new(Int64Type)), Vec::<i64>::new());
}

#[test]
fn list_is_send_sync() {
    fn assert_send_sync<T: Send + Sync>() {}
    assert_send_sync::<SerieType<Int64Type>>();
}
