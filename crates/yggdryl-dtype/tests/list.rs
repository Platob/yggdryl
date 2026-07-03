//! Integration tests for the `list` data type — the generic nested holder over one
//! value type.

use yggdryl_dtype::{
    arrow_schema, DataError, DataType, Int64, List, Nested, Optional, RawDataType, RawList,
    RawNested, TypedList,
};

#[test]
fn list_describes_itself_and_round_trips() {
    let list = List::new(Int64);
    assert_eq!(list.name(), "list");
    assert_eq!(list.arrow_format(), "+l");
    assert_eq!(list.byte_width(), None);
    assert_eq!(list.child_count(), 1);
    assert_eq!(list.value_type(), &Int64);

    assert_eq!(List::from_arrow(&list.to_arrow()).unwrap(), list);
    assert!(matches!(
        List::<Int64>::from_arrow(&arrow_schema::DataType::Int64),
        Err(DataError::IncompatibleArrowType { .. })
    ));
}

#[test]
fn list_codec_concatenates_elements() {
    let list = List::new(Int64);
    let bytes = list.native_to_bytes(&vec![1, 2, 3]);
    assert_eq!(bytes.len(), 24);
    assert_eq!(list.native_from_bytes(&bytes).unwrap(), vec![1, 2, 3]);
    assert_eq!(list.native_from_bytes(&[]).unwrap(), Vec::<i64>::new());

    // A remainder is a length error naming the nearest whole-element length.
    assert!(matches!(
        list.native_from_bytes(&[0; 9]),
        Err(DataError::InvalidByteLength {
            expected: 16,
            got: 9
        })
    ));

    // An optional element delegates its codec width to the value type, so the
    // round trip stays exact even though the *physical* width is indeterminate.
    let optional_list = List::new(Optional::new(Int64));
    let bytes = optional_list.native_to_bytes(&vec![1, 2]);
    assert_eq!(optional_list.native_from_bytes(&bytes).unwrap(), vec![1, 2]);

    // A variable-width element cannot be split from bytes.
    let nested = List::new(List::new(Int64));
    assert!(matches!(
        nested.native_from_bytes(&[0; 8]),
        Err(DataError::IndeterminateElementWidth { .. })
    ));
}

#[test]
fn list_is_the_generic_nested_holder() {
    // The typed pair: RawNested counts children, Nested<T> adds the native codec,
    // and TypedList pins the value type.
    fn raw_children<N: RawNested>(nested: &N) -> usize {
        nested.child_count()
    }
    fn typed_default<T, N: Nested<T>>(nested: &N) -> T {
        nested.default_value()
    }
    fn list_default<T, L: TypedList<T>>(list: &L) -> Vec<T> {
        list.default_value()
    }
    assert_eq!(raw_children(&List::new(Int64)), 1);
    assert_eq!(
        typed_default::<Vec<i64>, _>(&List::new(Int64)),
        Vec::<i64>::new()
    );
    assert_eq!(list_default(&List::new(Int64)), Vec::<i64>::new());
}

#[test]
fn list_is_send_sync() {
    fn assert_send_sync<T: Send + Sync>() {}
    assert_send_sync::<List<Int64>>();
}
