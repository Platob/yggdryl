//! Integration tests for [`AnySerie`] — the type-erased column: zero-copy
//! decomposition from Arrow, on-demand reconstitution, slicing, and equality
//! across representations.

use std::sync::Arc;

use yggdryl_scalar::arrow_array::{self, Array, ArrayRef};
use yggdryl_scalar::{AnySerie, Int64Serie};

// Every integer width decomposes into its concrete serie and reconstitutes the
// Arrow array around the same shared buffer — zero copy, both directions.
macro_rules! decompose_tests {
    ($test:ident, $variant:ident, $array:ident, $native:ty) => {
        #[test]
        fn $test() {
            let arrow: ArrayRef = Arc::new(arrow_array::$array::from(vec![
                1 as $native,
                2 as $native,
                3 as $native,
            ]));
            let serie = AnySerie::from_arrow(arrow.clone());
            let AnySerie::$variant(concrete) = &serie else {
                panic!("expected the decomposed {} variant", stringify!($variant));
            };
            // Shared, not copied: the serie's buffer is the Arrow array's buffer.
            let source = arrow
                .as_any()
                .downcast_ref::<arrow_array::$array>()
                .unwrap();
            assert_eq!(
                concrete.values().unwrap().as_ptr(),
                source.values().as_ptr()
            );

            // Reconstitution shares the same buffer too and compares equal.
            let round = serie.to_arrow();
            assert_eq!(round.as_ref(), arrow.as_ref());
            let round = round
                .as_any()
                .downcast_ref::<arrow_array::$array>()
                .unwrap();
            assert_eq!(round.values().as_ptr(), source.values().as_ptr());
        }
    };
}

decompose_tests!(int8_decomposes, Int8, Int8Array, i8);
decompose_tests!(int16_decomposes, Int16, Int16Array, i16);
decompose_tests!(int32_decomposes, Int32, Int32Array, i32);
decompose_tests!(int64_decomposes, Int64, Int64Array, i64);
decompose_tests!(uint8_decomposes, UInt8, UInt8Array, u8);
decompose_tests!(uint16_decomposes, UInt16, UInt16Array, u16);
decompose_tests!(uint32_decomposes, UInt32, UInt32Array, u32);
decompose_tests!(uint64_decomposes, UInt64, UInt64Array, u64);

#[test]
fn non_integer_types_pass_through_zero_copy() {
    // A binary array has no decomposed serie yet: the Arrow handle is kept as-is.
    let arrow: ArrayRef = Arc::new(arrow_array::BinaryArray::from_iter_values([
        b"a".as_slice(),
        b"bc".as_slice(),
    ]));
    let serie = AnySerie::from_arrow(arrow.clone());
    assert!(matches!(serie, AnySerie::Arrow(_)));
    assert_eq!(serie.len(), 2);
    assert_eq!(serie.to_arrow().as_ref(), arrow.as_ref());
    assert_eq!(serie.data_type(), *arrow.data_type());
}

#[test]
fn nulls_survive_the_decomposition() {
    let arrow: ArrayRef = Arc::new(arrow_array::Int64Array::from(vec![Some(1), None, Some(3)]));
    let serie = AnySerie::from_arrow(arrow.clone());
    let AnySerie::Int64(concrete) = &serie else {
        panic!("expected Int64");
    };
    assert_eq!(concrete.nulls().map(|nulls| nulls.null_count()), Some(1));
    assert_eq!(serie.to_arrow().as_ref(), arrow.as_ref());
}

#[test]
fn slices_stay_decomposed_and_share_buffers() {
    let serie = AnySerie::from(Int64Serie::from(vec![1, 2, 3, 4]));
    let window = serie.slice(1, 2);
    assert!(matches!(window, AnySerie::Int64(_)));
    assert_eq!(window.len(), 2);
    let AnySerie::Int64(window) = &window else {
        unreachable!()
    };
    assert_eq!(window.values(), Some(&[2, 3][..]));
    // The window borrows into the original buffer — zero copy.
    let AnySerie::Int64(original) = &serie else {
        unreachable!()
    };
    assert_eq!(
        window.values().unwrap().as_ptr(),
        original.values().unwrap()[1..].as_ptr()
    );
}

#[test]
fn equality_bridges_representations() {
    // A decomposed column equals its zero-copy passthrough twin.
    let arrow: ArrayRef = Arc::new(arrow_array::Int64Array::from(vec![1, 2, 3]));
    let decomposed = AnySerie::from(Int64Serie::from(vec![1, 2, 3]));
    let passthrough = AnySerie::Arrow(arrow);
    assert_eq!(decomposed, passthrough);
    assert_ne!(decomposed, AnySerie::from(Int64Serie::from(vec![1, 2, 4])));
}

#[test]
fn empty_columns_round_trip() {
    let serie = AnySerie::from(Int64Serie::from(Vec::<i64>::new()));
    assert!(serie.is_empty());
    assert_eq!(serie.to_arrow().len(), 0);
    assert_eq!(AnySerie::from_arrow(serie.to_arrow()), serie);
}
