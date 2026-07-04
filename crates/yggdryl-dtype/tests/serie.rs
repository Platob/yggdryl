//! Integration tests for the `serie` data type — the dynamic [`SerieType`] and the
//! statically-typed [`TypedSerieType`] over one value type.

use yggdryl_dtype::{
    arrow_schema, DataError, DataType, Int64Type, Nested, Serie, SerieType, TypedDataType,
    TypedNested, TypedOptionalType, TypedSerie, TypedSerieType,
};

#[test]
fn typed_serie_describes_itself_and_round_trips() {
    let serie = TypedSerieType::new(Int64Type);
    assert_eq!(serie.name(), "list");
    assert_eq!(serie.arrow_format(), "+l");
    assert_eq!(serie.byte_width(), None);
    assert_eq!(serie.child_count(), 1);
    assert_eq!(serie.value_type(), &Int64Type);
    assert_eq!(serie.item_field().name(), "item");

    assert_eq!(
        TypedSerieType::from_arrow(&serie.to_arrow()).unwrap(),
        serie
    );
    assert!(matches!(
        TypedSerieType::<Int64Type>::from_arrow(&arrow_schema::DataType::Int64),
        Err(DataError::IncompatibleArrowType { .. })
    ));
}

#[test]
fn dynamic_serie_is_arrow_backed_and_erases() {
    // The dynamic serie carries its child as an Arrow field, untyped.
    let serie = SerieType::new(arrow_schema::DataType::Int64);
    assert_eq!(serie.name(), "list");
    assert_eq!(serie.child_count(), 1);
    assert_eq!(serie.item_field().name(), "item");
    assert!(serie.item_field().is_nullable());

    // erase() and from_arrow agree; the round trip is lossless.
    assert_eq!(TypedSerieType::new(Int64Type).erase(), serie);
    assert_eq!(SerieType::from_arrow(&serie.to_arrow()).unwrap(), serie);
    assert!(matches!(
        SerieType::from_arrow(&arrow_schema::DataType::Int64),
        Err(DataError::IncompatibleArrowType { .. })
    ));
}

#[test]
fn serie_codec_concatenates_elements() {
    let serie = TypedSerieType::new(Int64Type);
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
    let optional_list = TypedSerieType::new(TypedOptionalType::new(Int64Type));
    let bytes = optional_list.native_to_bytes(&vec![1, 2]);
    assert_eq!(optional_list.native_from_bytes(&bytes).unwrap(), vec![1, 2]);

    // A variable-width element cannot be split from bytes.
    let nested = TypedSerieType::new(TypedSerieType::new(Int64Type));
    assert!(matches!(
        nested.native_from_bytes(&[0; 8]),
        Err(DataError::IndeterminateElementWidth { .. })
    ));
}

#[test]
fn serie_is_the_generic_nested_holder() {
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
    assert_eq!(raw_children(&TypedSerieType::new(Int64Type)), 1);
    assert_eq!(
        raw_children(&SerieType::new(arrow_schema::DataType::Int64)),
        1
    );
    assert_eq!(
        typed_default::<Vec<i64>, _>(&TypedSerieType::new(Int64Type)),
        Vec::<i64>::new()
    );
    assert_eq!(
        serie_default(&TypedSerieType::new(Int64Type)),
        Vec::<i64>::new()
    );
}

#[test]
fn serie_is_send_sync() {
    fn assert_send_sync<T: Send + Sync>() {}
    assert_send_sync::<SerieType>();
    assert_send_sync::<TypedSerieType<Int64Type>>();
}
